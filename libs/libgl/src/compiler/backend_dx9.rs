//! IR → DX9 Shader Model 2.0 bytecode compiler backend.
//!
//! Translates the libgl IR instruction set into DX9 SM 2.0 bytecode
//! suitable for SVGA3D `SHADER_DEFINE` commands.
//!
//! # DX9 SM 2.0 bytecode format
//!
//! Reference: <https://learn.microsoft.com/en-us/windows-hardware/drivers/display/shader-code-format>
//!
//! A shader is a sequence of 32-bit DWORDs:
//! ```text
//! [VERSION_TOKEN] [DCL instructions...] [DEF instructions...] [shader body...] [END_TOKEN]
//! ```
//!
//! ## Version tokens
//! - VS 2.0: `0xFFFE0200`
//! - PS 2.0: `0xFFFF0200`
//! - End:    `0x0000FFFF`
//!
//! ## Instruction token (DWORD 0 of each instruction)
//! - `[15:0]`  = opcode (`D3DSIO_*`)
//! - `[27:24]` = instruction length in DWORDs (SM 2.0+, excluding this token)
//! - `[31]`    = 0
//!
//! ## Register parameter token (destination or source)
//! - `[10:0]`  = register number (0–2047)
//! - `[12:11]` = register type bits [4:3] (high 2 bits of `D3DSPR_*`)
//! - `[15:14]` = reserved (0)
//! - `[19:16]` = write mask (dest) or swizzle X/Y (src)
//! - `[23:20]` = result modifier (dest) or swizzle Z/W (src)
//! - `[27:24]` = source modifier (`D3DSPSM_*`, src only)
//! - `[30:28]` = register type bits [2:0] (low 3 bits of `D3DSPR_*`)
//! - `[31]`    = 1 (sentinel bit, always set)
//!
//! ## Register types (`D3DSPR_*`)
//! | Value | Name       | Usage                                          |
//! |-------|------------|-------------------------------------------------|
//! |   0   | TEMP       | `r#` temporaries                               |
//! |   1   | INPUT      | `v#` (VS input / PS interpolated)              |
//! |   2   | CONST      | `c#` float constants                           |
//! |   3   | ADDR/TEX   | `a0` (VS) / `t#` (PS)                         |
//! |   4   | RASTOUT    | `oPos`(0) `oFog`(1) `oPts`(2) — VS only       |
//! |   5   | ATTROUT    | `oD0` `oD1` — VS color output                 |
//! |   6   | TEXCRDOUT  | `oT0`–`oT7` — VS texcoord output (SM < 3.0)   |
//! |   8   | COLOROUT   | `oC0`–`oC3` — PS color output                 |
//! |   9   | DEPTHOUT   | `oDepth` — PS depth output                    |
//! |  10   | SAMPLER    | `s#` sampler registers                         |
//!
//! ## Instruction opcodes (`D3DSIO_*`)
//! | Value | Name   | Args | Description                     |
//! |-------|--------|------|---------------------------------|
//! |   1   | MOV    | 2    | dst = src                       |
//! |   2   | ADD    | 3    | dst = src0 + src1               |
//! |   3   | SUB    | 3    | dst = src0 - src1               |
//! |   4   | MAD    | 4    | dst = src0 * src1 + src2        |
//! |   5   | MUL    | 3    | dst = src0 * src1               |
//! |   6   | RCP    | 2    | dst = 1 / src                   |
//! |   7   | RSQ    | 2    | dst = 1 / sqrt(src)             |
//! |   8   | DP3    | 3    | dst = dot3(src0, src1)          |
//! |   9   | DP4    | 3    | dst = dot4(src0, src1)          |
//! |  10   | MIN    | 3    | dst = min(src0, src1)           |
//! |  11   | MAX    | 3    | dst = max(src0, src1)           |
//! |  12   | SLT    | 3    | dst = (src0 < src1) ? 1 : 0    |
//! |  13   | SGE    | 3    | dst = (src0 >= src1) ? 1 : 0   |
//! |  18   | LRP    | 4    | dst = src0*src1 + (1-src0)*src2 |
//! |  19   | FRC    | 2    | dst = frac(src)                 |
//! |  31   | DCL    | special | input/sampler declaration    |
//! |  32   | POW    | 3    | dst = pow(src0, src1)           |
//! |  33   | CRS    | 3    | dst = cross(src0, src1)         |
//! |  35   | ABS    | 2    | dst = abs(src)                  |
//! |  36   | NRM    | 2    | dst = normalize(src)            |
//! |  37   | SINCOS | 4*   | dst.xy = (cos, sin) of src0     |
//! |  66   | TEX    | 3    | dst = texture(sampler, coord)   |
//! |  81   | DEF    | 5    | define inline float constant    |
//!
//! \* SINCOS takes 4 args in SM 2.0 (dst, src, const1, const2), 2 in SM 3.0.
//!
//! ## DCL format
//! - **VS input:** `[DCL] [(1<<31)|usage|(idx<<16)] [dst_token(INPUT, reg)]`
//! - **PS input:** `[DCL] [(1<<31)]                  [dst_token(INPUT, reg)]`
//! - **PS sampler:** `[DCL] [(1<<31)|(textype<<27)]  [dst_token(SAMPLER, reg)]`
//!
//! ## Write mask bits (destination)
//! Bit 16=X, 17=Y, 18=Z, 19=W. All=`0x000F0000`.
//!
//! ## Swizzle (source, bits [23:16])
//! Each 2-bit field selects a component: X=0, Y=1, Z=2, W=3.
//! Identity `.xyzw` = `0xE4 << 16`.

use alloc::vec::Vec;
use super::ir::{self, Inst, Reg};

// ── DX9 version tokens ──────────────────────────────────

const VS_2_0: u32 = 0xFFFE_0200;
const PS_2_0: u32 = 0xFFFF_0200;
const END_TOKEN: u32 = 0x0000_FFFF;

// ── DX9 instruction opcodes ─────────────────────────────

const D3DSIO_NOP: u32    = 0;
const D3DSIO_MOV: u32    = 1;
const D3DSIO_ADD: u32    = 2;
const D3DSIO_SUB: u32    = 3;
const D3DSIO_MAD: u32    = 4;
const D3DSIO_MUL: u32    = 5;
const D3DSIO_RCP: u32    = 6;
const D3DSIO_RSQ: u32    = 7;
const D3DSIO_DP3: u32    = 8;
const D3DSIO_DP4: u32    = 9;
const D3DSIO_MIN: u32    = 10;
const D3DSIO_MAX: u32    = 11;
const D3DSIO_SLT: u32    = 12;
const D3DSIO_SGE: u32    = 13;
const D3DSIO_EXP: u32    = 14;
const D3DSIO_LOG: u32    = 15;
const D3DSIO_LRP: u32    = 18;
const D3DSIO_FRC: u32    = 19;
const D3DSIO_M4X4: u32   = 20;
const D3DSIO_M4X3: u32   = 21;
const D3DSIO_M3X3: u32   = 23;
const D3DSIO_DCL: u32    = 31;
const D3DSIO_POW: u32    = 32;
const D3DSIO_CRS: u32    = 33;
const D3DSIO_NRM: u32    = 36;
const D3DSIO_SINCOS: u32 = 37;
const D3DSIO_ABS: u32    = 35;
const D3DSIO_DEF: u32    = 81;
const D3DSIO_TEXLD: u32  = 66;

// ── Register types ──────────────────────────────────────

const D3DSPR_TEMP: u32     = 0;   // r registers (temporaries)
const D3DSPR_INPUT: u32    = 1;   // v registers (VS input / PS interpolated)
const D3DSPR_CONST: u32    = 2;   // c registers (shader constants)
const D3DSPR_RASTOUT: u32  = 4;   // VS rasterizer output: oPos(0), oFog(1), oPts(2)
const D3DSPR_ATTROUT: u32  = 5;   // VS color output: oD0, oD1
const D3DSPR_TEXCRDOUT: u32 = 6;  // VS texcoord output: oT0-oT7
const D3DSPR_COLOROUT: u32 = 8;   // PS color output: oC0-oC3
const D3DSPR_SAMPLER: u32  = 10;  // s registers (PS texture sampler)

// ── Write mask bits ─────────────────────────────────────

const D3DSP_WRITEMASK_0: u32 = 1 << 16; // .x
const D3DSP_WRITEMASK_1: u32 = 1 << 17; // .y
const D3DSP_WRITEMASK_2: u32 = 1 << 18; // .z
const D3DSP_WRITEMASK_3: u32 = 1 << 19; // .w
const D3DSP_WRITEMASK_ALL: u32 =
    D3DSP_WRITEMASK_0 | D3DSP_WRITEMASK_1 | D3DSP_WRITEMASK_2 | D3DSP_WRITEMASK_3;

// ── Source swizzle ──────────────────────────────────────

/// Identity swizzle: .xyzw = 0b_11_10_01_00 = 0xE4
const SWIZZLE_IDENTITY: u32 = 0xE4;

// Source modifier: negate
const D3DSPSM_NEG: u32 = 1 << 24;

// ── Token encoding helpers ──────────────────────────────

/// Encode a destination register token.
fn dst_token(reg_type: u32, reg_num: u32, write_mask: u32) -> u32 {
    let type_lo = (reg_type & 0x7) << 28;
    let type_hi = ((reg_type >> 3) & 0x3) << 11;
    (1u32 << 31) | type_lo | type_hi | (reg_num & 0x7FF) | write_mask
}

/// Encode a source register token with identity swizzle.
fn src_token(reg_type: u32, reg_num: u32) -> u32 {
    let type_lo = (reg_type & 0x7) << 28;
    let type_hi = ((reg_type >> 3) & 0x3) << 11;
    let swizzle = SWIZZLE_IDENTITY << 16;
    (1u32 << 31) | type_lo | type_hi | (reg_num & 0x7FF) | swizzle
}

/// Encode a source register token with negate modifier.
fn src_token_neg(reg_type: u32, reg_num: u32) -> u32 {
    src_token(reg_type, reg_num) | D3DSPSM_NEG
}

/// Encode a source register with a broadcast swizzle (all components = component).
fn src_token_broadcast(reg_type: u32, reg_num: u32, component: u32) -> u32 {
    let type_lo = (reg_type & 0x7) << 28;
    let type_hi = ((reg_type >> 3) & 0x3) << 11;
    let swz = component & 3;
    let swizzle = (swz | (swz << 2) | (swz << 4) | (swz << 6)) << 16;
    (1u32 << 31) | type_lo | type_hi | (reg_num & 0x7FF) | swizzle
}

// ── Compilation context ─────────────────────────────────

/// State for collecting LoadConst values that need to be emitted as DEF instructions.
struct CompileCtx {
    /// Bytecode output.
    bc: Vec<u32>,
    /// Number of temp registers used by IR (for scratch allocation).
    num_ir_regs: u32,
    /// Next available scratch temp register.
    next_scratch: u32,
}

impl CompileCtx {
    fn new(num_ir_regs: u32) -> Self {
        Self {
            bc: Vec::with_capacity(256),
            num_ir_regs,
            // Scratch registers start after all IR registers + 2 extras
            next_scratch: num_ir_regs + 2,
        }
    }

    /// Allocate a scratch temp register (for multi-instruction decomposition).
    fn scratch(&mut self) -> u32 {
        let r = self.next_scratch;
        self.next_scratch += 1;
        r
    }

    // ── Instruction emission helpers ─────────────────────

    fn emit_1dst_0src(&mut self, opcode: u32, dst_type: u32, dst: u32) {
        self.bc.push(opcode);
        self.bc.push(dst_token(dst_type, dst, D3DSP_WRITEMASK_ALL));
    }

    fn emit_1dst_1src(&mut self, opcode: u32, dst: u32, src_type: u32, src: u32) {
        self.bc.push(opcode);
        self.bc.push(dst_token(D3DSPR_TEMP, dst, D3DSP_WRITEMASK_ALL));
        self.bc.push(src_token(src_type, src));
    }

    fn emit_mov(&mut self, dst_type: u32, dst: u32, src_type: u32, src: u32) {
        self.bc.push(D3DSIO_MOV);
        self.bc.push(dst_token(dst_type, dst, D3DSP_WRITEMASK_ALL));
        self.bc.push(src_token(src_type, src));
    }

    fn emit_2src(&mut self, opcode: u32, dst: u32, a_type: u32, a: u32, b_type: u32, b: u32) {
        self.bc.push(opcode);
        self.bc.push(dst_token(D3DSPR_TEMP, dst, D3DSP_WRITEMASK_ALL));
        self.bc.push(src_token(a_type, a));
        self.bc.push(src_token(b_type, b));
    }

    fn emit_3src(&mut self, opcode: u32, dst: u32, a_type: u32, a: u32, b_type: u32, b: u32, c_type: u32, c: u32) {
        self.bc.push(opcode);
        self.bc.push(dst_token(D3DSPR_TEMP, dst, D3DSP_WRITEMASK_ALL));
        self.bc.push(src_token(a_type, a));
        self.bc.push(src_token(b_type, b));
        self.bc.push(src_token(c_type, c));
    }

    /// Emit a DEF instruction (define a constant register inline).
    fn emit_def(&mut self, reg: u32, x: f32, y: f32, z: f32, w: f32) {
        self.bc.push(D3DSIO_DEF);
        self.bc.push(dst_token(D3DSPR_CONST, reg, D3DSP_WRITEMASK_ALL));
        self.bc.push(x.to_bits());
        self.bc.push(y.to_bits());
        self.bc.push(z.to_bits());
        self.bc.push(w.to_bits());
    }
}

// ══════════════════════════════════════════════════════════
// Public API
// ══════════════════════════════════════════════════════════

/// Compile an IR program to DX9 SM 2.0 bytecode.
///
/// `is_vertex`: true for vertex shader, false for pixel shader.
///
/// Returns the bytecode as a `Vec<u32>` ready for SVGA3D `SHADER_DEFINE`.
/// Also returns a list of (const_reg, [f32; 4]) pairs that must be uploaded
/// via `SET_SHADER_CONST` before drawing.
pub fn compile(program: &ir::Program, is_vertex: bool) -> (Vec<u32>, Vec<(u32, [f32; 4])>) {
    let mut ctx = CompileCtx::new(program.num_regs);
    let mut constants: Vec<(u32, [f32; 4])> = Vec::new();

    // Next available constant register (after uniforms)
    let mut next_const_reg = program.uniforms.len() as u32;

    // Version token
    ctx.bc.push(if is_vertex { VS_2_0 } else { PS_2_0 });

    // Emit DCL instructions for inputs
    if is_vertex {
        // Declare vertex shader inputs
        for (i, attr) in program.attributes.iter().enumerate() {
            // DCL usage token: position=0, texcoord=5, etc.
            // For simplicity, declare all as TEXCOORD[i] except first which is POSITION
            let usage = if i == 0 { 0u32 } else { 5u32 }; // POSITION=0, TEXCOORD=5
            let usage_idx = if i == 0 { 0 } else { (i - 1) as u32 };
            ctx.bc.push(D3DSIO_DCL);
            ctx.bc.push((1u32 << 31) | (usage & 0xF) | ((usage_idx & 0xF) << 16));
            ctx.bc.push(dst_token(D3DSPR_INPUT, i as u32, D3DSP_WRITEMASK_ALL));
        }
    } else {
        // Declare pixel shader input registers (interpolated varyings)
        // PS 2.0 DCL: usage token = just the sentinel bit (no usage/semantic data)
        for (i, _var) in program.varyings.iter().enumerate() {
            ctx.bc.push(D3DSIO_DCL);
            ctx.bc.push(1u32 << 31); // PS 2.0: no usage/semantic, just sentinel
            ctx.bc.push(dst_token(D3DSPR_INPUT, i as u32, D3DSP_WRITEMASK_ALL));
        }
    }

    // First pass: collect LoadConst values and assign constant registers
    for inst in &program.instructions {
        if let Inst::LoadConst(dst, values) = inst {
            let creg = next_const_reg;
            next_const_reg += 1;
            constants.push((creg, *values));
            // Emit DEF c_reg, values (inline constant definition)
            ctx.emit_def(creg, values[0], values[1], values[2], values[3]);
        }
    }

    // Build a map from IR registers used by LoadConst to their constant register
    let mut const_map: Vec<(Reg, u32)> = Vec::new();
    let mut ci = 0;
    for inst in &program.instructions {
        if let Inst::LoadConst(dst, _) = inst {
            const_map.push((*dst, constants[ci].0));
            ci += 1;
        }
    }

    // Second pass: emit instructions
    for inst in &program.instructions {
        emit_inst(&mut ctx, inst, program, is_vertex, &const_map);
    }

    ctx.bc.push(END_TOKEN);
    (ctx.bc, constants)
}

/// Look up if this IR register was assigned a constant register via LoadConst.
fn lookup_const(reg: Reg, const_map: &[(Reg, u32)]) -> Option<u32> {
    for &(ir_reg, creg) in const_map {
        if ir_reg == reg { return Some(creg); }
    }
    None
}

/// Get the source register type and number for an IR register.
///
/// Always returns the temp register — LoadConst already emits `MOV r, c`,
/// so the temp register always holds the correct (potentially modified) value.
/// Using const_map here would be wrong when the register is later modified
/// by WriteMask or other instructions.
fn ir_src(reg: Reg, _const_map: &[(Reg, u32)]) -> (u32, u32) {
    (D3DSPR_TEMP, reg)
}

fn emit_inst(ctx: &mut CompileCtx, inst: &Inst, program: &ir::Program, is_vertex: bool, const_map: &[(Reg, u32)]) {
    match inst {
        Inst::LoadConst(dst, values) => {
            // Already handled by DEF instructions + const_map.
            // Emit a MOV from the constant register to the temp register.
            if let Some(creg) = lookup_const(*dst, const_map) {
                ctx.emit_mov(D3DSPR_TEMP, *dst, D3DSPR_CONST, creg);
            }
        }

        Inst::Mov(dst, src) => {
            let (st, sn) = ir_src(*src, const_map);
            ctx.emit_mov(D3DSPR_TEMP, *dst, st, sn);
        }

        Inst::Add(dst, a, b) => {
            let (at, an) = ir_src(*a, const_map);
            let (bt, bn) = ir_src(*b, const_map);
            ctx.emit_2src(D3DSIO_ADD, *dst, at, an, bt, bn);
        }

        Inst::Sub(dst, a, b) => {
            let (at, an) = ir_src(*a, const_map);
            let (bt, bn) = ir_src(*b, const_map);
            ctx.emit_2src(D3DSIO_SUB, *dst, at, an, bt, bn);
        }

        Inst::Mul(dst, a, b) => {
            let (at, an) = ir_src(*a, const_map);
            let (bt, bn) = ir_src(*b, const_map);
            ctx.emit_2src(D3DSIO_MUL, *dst, at, an, bt, bn);
        }

        Inst::Div(dst, a, b) => {
            // DX9 SM 2.0 has no DIV. Decompose: RCP temp, b; MUL dst, a, temp
            let (at, an) = ir_src(*a, const_map);
            let (bt, bn) = ir_src(*b, const_map);
            let tmp = ctx.scratch();
            ctx.emit_1dst_1src(D3DSIO_RCP, tmp, bt, bn);
            ctx.emit_2src(D3DSIO_MUL, *dst, at, an, D3DSPR_TEMP, tmp);
        }

        Inst::Neg(dst, src) => {
            let (st, sn) = ir_src(*src, const_map);
            ctx.bc.push(D3DSIO_MOV);
            ctx.bc.push(dst_token(D3DSPR_TEMP, *dst, D3DSP_WRITEMASK_ALL));
            ctx.bc.push(src_token_neg(st, sn));
        }

        Inst::Dp3(dst, a, b) => {
            let (at, an) = ir_src(*a, const_map);
            let (bt, bn) = ir_src(*b, const_map);
            ctx.emit_2src(D3DSIO_DP3, *dst, at, an, bt, bn);
        }

        Inst::Dp4(dst, a, b) => {
            let (at, an) = ir_src(*a, const_map);
            let (bt, bn) = ir_src(*b, const_map);
            ctx.emit_2src(D3DSIO_DP4, *dst, at, an, bt, bn);
        }

        Inst::Cross(dst, a, b) => {
            let (at, an) = ir_src(*a, const_map);
            let (bt, bn) = ir_src(*b, const_map);
            ctx.emit_2src(D3DSIO_CRS, *dst, at, an, bt, bn);
        }

        Inst::Normalize(dst, src) => {
            let (st, sn) = ir_src(*src, const_map);
            ctx.emit_1dst_1src(D3DSIO_NRM, *dst, st, sn);
        }

        Inst::Length(dst, src) => {
            // length(v) = sqrt(dot(v,v))
            // DP3 tmp, src, src  → tmp.x = dot product
            // RSQ tmp2, tmp      → tmp2.x = 1/sqrt(dp)
            // RCP dst, tmp2      → dst.x = sqrt(dp)
            let (st, sn) = ir_src(*src, const_map);
            let tmp1 = ctx.scratch();
            let tmp2 = ctx.scratch();
            ctx.emit_2src(D3DSIO_DP3, tmp1, st, sn, st, sn);
            ctx.emit_1dst_1src(D3DSIO_RSQ, tmp2, D3DSPR_TEMP, tmp1);
            ctx.emit_1dst_1src(D3DSIO_RCP, *dst, D3DSPR_TEMP, tmp2);
        }

        Inst::Min(dst, a, b) => {
            let (at, an) = ir_src(*a, const_map);
            let (bt, bn) = ir_src(*b, const_map);
            ctx.emit_2src(D3DSIO_MIN, *dst, at, an, bt, bn);
        }

        Inst::Max(dst, a, b) => {
            let (at, an) = ir_src(*a, const_map);
            let (bt, bn) = ir_src(*b, const_map);
            ctx.emit_2src(D3DSIO_MAX, *dst, at, an, bt, bn);
        }

        Inst::Clamp(dst, x, lo, hi) => {
            // clamp(x, lo, hi) = max(lo, min(x, hi))
            let (xt, xn) = ir_src(*x, const_map);
            let (lt, ln) = ir_src(*lo, const_map);
            let (ht, hn) = ir_src(*hi, const_map);
            let tmp = ctx.scratch();
            ctx.emit_2src(D3DSIO_MIN, tmp, xt, xn, ht, hn);
            ctx.emit_2src(D3DSIO_MAX, *dst, D3DSPR_TEMP, tmp, lt, ln);
        }

        Inst::Mix(dst, a, b, t) => {
            // DX9 LRP: dst = t*a + (1-t)*b  (note: GLSL mix(a,b,t) = a*(1-t) + b*t)
            // So LRP(t, b, a) = t*b + (1-t)*a = mix(a, b, t)
            let (at, an) = ir_src(*a, const_map);
            let (bt, bn) = ir_src(*b, const_map);
            let (tt, tn) = ir_src(*t, const_map);
            ctx.emit_3src(D3DSIO_LRP, *dst, tt, tn, bt, bn, at, an);
        }

        Inst::Abs(dst, src) => {
            let (st, sn) = ir_src(*src, const_map);
            ctx.bc.push(D3DSIO_ABS);
            ctx.bc.push(dst_token(D3DSPR_TEMP, *dst, D3DSP_WRITEMASK_ALL));
            ctx.bc.push(src_token(st, sn));
        }

        Inst::Floor(dst, src) => {
            // floor(x) = x - frac(x)
            let (st, sn) = ir_src(*src, const_map);
            let tmp = ctx.scratch();
            ctx.emit_1dst_1src(D3DSIO_FRC, tmp, st, sn);
            ctx.emit_2src(D3DSIO_SUB, *dst, st, sn, D3DSPR_TEMP, tmp);
        }

        Inst::Fract(dst, src) => {
            let (st, sn) = ir_src(*src, const_map);
            ctx.emit_1dst_1src(D3DSIO_FRC, *dst, st, sn);
        }

        Inst::Pow(dst, base, exp) => {
            let (bt, bn) = ir_src(*base, const_map);
            let (et, en) = ir_src(*exp, const_map);
            ctx.emit_2src(D3DSIO_POW, *dst, bt, bn, et, en);
        }

        Inst::Sqrt(dst, src) => {
            // sqrt(x) = x * rsqrt(x)
            let (st, sn) = ir_src(*src, const_map);
            let tmp = ctx.scratch();
            ctx.emit_1dst_1src(D3DSIO_RSQ, tmp, st, sn);
            ctx.emit_2src(D3DSIO_MUL, *dst, st, sn, D3DSPR_TEMP, tmp);
        }

        Inst::Rsqrt(dst, src) => {
            let (st, sn) = ir_src(*src, const_map);
            ctx.emit_1dst_1src(D3DSIO_RSQ, *dst, st, sn);
        }

        Inst::Sin(dst, src) => {
            // SINCOS dst.xy, src  (dst.x = cos, dst.y = sin)
            // We want sin → use .y component
            let (st, sn) = ir_src(*src, const_map);
            let tmp = ctx.scratch();
            ctx.bc.push(D3DSIO_SINCOS);
            ctx.bc.push(dst_token(D3DSPR_TEMP, tmp, D3DSP_WRITEMASK_0 | D3DSP_WRITEMASK_1));
            ctx.bc.push(src_token(st, sn));
            // MOV dst, tmp.yyyy (broadcast sin result)
            ctx.bc.push(D3DSIO_MOV);
            ctx.bc.push(dst_token(D3DSPR_TEMP, *dst, D3DSP_WRITEMASK_ALL));
            ctx.bc.push(src_token_broadcast(D3DSPR_TEMP, tmp, 1)); // .y = sin
        }

        Inst::Cos(dst, src) => {
            // SINCOS dst.xy, src  (dst.x = cos, dst.y = sin)
            let (st, sn) = ir_src(*src, const_map);
            let tmp = ctx.scratch();
            ctx.bc.push(D3DSIO_SINCOS);
            ctx.bc.push(dst_token(D3DSPR_TEMP, tmp, D3DSP_WRITEMASK_0 | D3DSP_WRITEMASK_1));
            ctx.bc.push(src_token(st, sn));
            // MOV dst, tmp.xxxx (broadcast cos result)
            ctx.bc.push(D3DSIO_MOV);
            ctx.bc.push(dst_token(D3DSPR_TEMP, *dst, D3DSP_WRITEMASK_ALL));
            ctx.bc.push(src_token_broadcast(D3DSPR_TEMP, tmp, 0)); // .x = cos
        }

        Inst::Reflect(dst, i, n) => {
            // reflect(I, N) = I - 2 * dot(N, I) * N
            // DP3 tmp, N, I
            // ADD tmp, tmp, tmp       (tmp = 2*dot)
            // MUL tmp2, N, tmp        (tmp2 = 2*dot*N)
            // SUB dst, I, tmp2
            let (it, in_) = ir_src(*i, const_map);
            let (nt, nn) = ir_src(*n, const_map);
            let tmp = ctx.scratch();
            let tmp2 = ctx.scratch();
            ctx.emit_2src(D3DSIO_DP3, tmp, nt, nn, it, in_);
            ctx.emit_2src(D3DSIO_ADD, tmp, D3DSPR_TEMP, tmp, D3DSPR_TEMP, tmp);
            ctx.emit_2src(D3DSIO_MUL, tmp2, nt, nn, D3DSPR_TEMP, tmp);
            ctx.emit_2src(D3DSIO_SUB, *dst, it, in_, D3DSPR_TEMP, tmp2);
        }

        Inst::TexSample(dst, sampler, coord) => {
            let (ct, cn) = ir_src(*coord, const_map);
            ctx.bc.push(D3DSIO_TEXLD);
            ctx.bc.push(dst_token(D3DSPR_TEMP, *dst, D3DSP_WRITEMASK_ALL));
            ctx.bc.push(src_token(ct, cn));
            ctx.bc.push(src_token(D3DSPR_SAMPLER, *sampler));
        }

        Inst::MatMul4(dst, mat, vec) => {
            // M4x4: dst, vec, mat[0]
            // mat occupies 4 consecutive registers starting at `mat`
            let (vt, vn) = ir_src(*vec, const_map);
            let (mt, mn) = ir_src(*mat, const_map);
            ctx.bc.push(D3DSIO_M4X4);
            ctx.bc.push(dst_token(D3DSPR_TEMP, *dst, D3DSP_WRITEMASK_ALL));
            ctx.bc.push(src_token(vt, vn));
            ctx.bc.push(src_token(mt, mn));
        }

        Inst::MatMul3(dst, mat, vec) => {
            let (vt, vn) = ir_src(*vec, const_map);
            let (mt, mn) = ir_src(*mat, const_map);
            ctx.bc.push(D3DSIO_M3X3);
            ctx.bc.push(dst_token(D3DSPR_TEMP, *dst, D3DSP_WRITEMASK_0 | D3DSP_WRITEMASK_1 | D3DSP_WRITEMASK_2));
            ctx.bc.push(src_token(vt, vn));
            ctx.bc.push(src_token(mt, mn));
        }

        Inst::Swizzle(dst, src, components, count) => {
            let (st, sn) = ir_src(*src, const_map);
            let swz = (components[0] as u32)
                | ((components[1] as u32) << 2)
                | ((components[2] as u32) << 4)
                | ((components[3] as u32) << 6);
            let type_lo = (st & 0x7) << 28;
            let type_hi = ((st >> 3) & 0x3) << 11;
            let src_tok = (1u32 << 31) | type_lo | type_hi | (sn & 0x7FF) | (swz << 16);
            ctx.bc.push(D3DSIO_MOV);
            ctx.bc.push(dst_token(D3DSPR_TEMP, *dst, D3DSP_WRITEMASK_ALL));
            ctx.bc.push(src_tok);
        }

        Inst::WriteMask(dst, src, mask) => {
            let (st, sn) = ir_src(*src, const_map);
            let wm = (*mask as u32 & 0xF) << 16;
            ctx.bc.push(D3DSIO_MOV);
            ctx.bc.push(dst_token(D3DSPR_TEMP, *dst, wm));
            ctx.bc.push(src_token(st, sn));
        }

        Inst::CmpLt(dst, a, b) => {
            let (at, an) = ir_src(*a, const_map);
            let (bt, bn) = ir_src(*b, const_map);
            ctx.emit_2src(D3DSIO_SLT, *dst, at, an, bt, bn);
        }

        Inst::CmpEq(dst, a, b) => {
            // No direct CmpEq in SM 2.0. Use: SGE(a,b) * SGE(b,a)
            let (at, an) = ir_src(*a, const_map);
            let (bt, bn) = ir_src(*b, const_map);
            let tmp = ctx.scratch();
            ctx.emit_2src(D3DSIO_SGE, *dst, at, an, bt, bn);
            ctx.emit_2src(D3DSIO_SGE, tmp, bt, bn, at, an);
            ctx.emit_2src(D3DSIO_MUL, *dst, D3DSPR_TEMP, *dst, D3DSPR_TEMP, tmp);
        }

        Inst::Select(dst, cond, a, b) => {
            // select(cond, a, b) = cond != 0 ? a : b
            // Use LRP: dst = cond * a + (1-cond) * b  (works if cond is 0.0 or 1.0)
            let (ct, cn) = ir_src(*cond, const_map);
            let (at, an) = ir_src(*a, const_map);
            let (bt, bn) = ir_src(*b, const_map);
            ctx.emit_3src(D3DSIO_LRP, *dst, ct, cn, at, an, bt, bn);
        }

        Inst::IntToFloat(dst, src) | Inst::FloatToInt(dst, src) => {
            // No-op in DX9 (same representation in registers)
            let (st, sn) = ir_src(*src, const_map);
            ctx.emit_mov(D3DSPR_TEMP, *dst, st, sn);
        }

        // ── I/O instructions ─────────────────────────────

        Inst::LoadAttribute(dst, idx) => {
            ctx.emit_mov(D3DSPR_TEMP, *dst, D3DSPR_INPUT, *idx);
        }

        Inst::LoadUniform(dst, idx) => {
            ctx.emit_mov(D3DSPR_TEMP, *dst, D3DSPR_CONST, *idx);
        }

        Inst::LoadVarying(dst, idx) => {
            // In pixel shader, varyings come from v registers (interpolated)
            ctx.emit_mov(D3DSPR_TEMP, *dst, D3DSPR_INPUT, *idx);
        }

        Inst::StorePosition(src) => {
            // Vertex shader: MOV oPos, r_src
            // SM 2.0: oPos = D3DSPR_RASTOUT, register 0
            let (st, sn) = ir_src(*src, const_map);
            ctx.emit_mov(D3DSPR_RASTOUT, 0, st, sn);
        }

        Inst::StoreFragColor(src) => {
            // Pixel shader: MOV oC0, r_src
            let (st, sn) = ir_src(*src, const_map);
            ctx.emit_mov(D3DSPR_COLOROUT, 0, st, sn);
        }

        Inst::StorePointSize(src) => {
            // Not supported in Phase 2, skip
        }

        Inst::StoreVarying(idx, src) => {
            // Vertex shader: output to texture coordinate register oTn
            // SM 2.0: oT0 = D3DSPR_TEXCRDOUT register 0, oT1 = register 1, etc.
            let (st, sn) = ir_src(*src, const_map);
            ctx.emit_mov(D3DSPR_TEXCRDOUT, *idx, st, sn);
        }
    }
}
