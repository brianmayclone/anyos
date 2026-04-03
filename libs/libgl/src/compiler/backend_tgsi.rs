//! IR → TGSI text backend for virgl GPU acceleration.
//!
//! Translates the libgl IR instruction set into TGSI assembly text that
//! virglrenderer can compile on the host side.
//!
//! TGSI text format example:
//! ```text
//! VERT
//! DCL IN[0]
//! DCL OUT[0], POSITION
//! DCL OUT[1], GENERIC[0]
//! DCL CONST[0..3]
//! DCL TEMP[0..2]
//!   0: MOV TEMP[0], IN[0]
//!   1: DP4 OUT[0].x, CONST[0], TEMP[0]
//!   ...
//!   N: END
//! ```

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt::Write;

use super::ir::{Inst, Program, Reg};

/// Compile an IR program to TGSI text.
///
/// `is_vertex`: true for vertex shader, false for fragment shader.
/// `num_samplers`: number of sampler uniforms (for SAMP declarations).
/// `const_base`: slot offset added to all CONST[] references (used to give FS its
///   correct position in the shared combined constant buffer; 0 for VS).
///
/// Returns the TGSI text as a String.
pub fn compile(program: &Program, is_vertex: bool, num_samplers: u32, const_base: u32) -> String {
    let mut ctx = TgsiCtx::new(program, is_vertex, num_samplers, const_base);
    ctx.emit_header();
    ctx.emit_declarations();
    ctx.emit_immediates();
    ctx.emit_body();
    ctx.emit_end();
    ctx.out
}

struct TgsiCtx<'a> {
    prog: &'a Program,
    is_vertex: bool,
    num_samplers: u32,
    out: String,
    /// Instruction counter for TGSI line numbering.
    pc: u32,
    /// Collected immediate values from LoadConst instructions.
    immediates: Vec<[f32; 4]>,
    /// Maps IR register → immediate index (if it was a LoadConst).
    imm_map: Vec<Option<u32>>,
    /// Number of TEMP registers needed.
    num_temps: u32,
    /// Number of CONST registers (uniform slots) used by this shader.
    num_consts: u32,
    /// Slot offset into the shared combined constant buffer.
    /// 0 for VS; = number of VS vec4 slots for FS.
    const_base: u32,
    /// Maps IR register → sampler unit index for TexSample.
    sampler_map: Vec<u32>,
}

impl<'a> TgsiCtx<'a> {
    fn new(prog: &'a Program, is_vertex: bool, num_samplers: u32, const_base: u32) -> Self {
        // Collect immediates from LoadConst instructions
        let mut immediates = Vec::new();
        let mut imm_map = Vec::new();
        imm_map.resize(prog.num_regs as usize, None);

        for inst in &prog.instructions {
            if let Inst::LoadConst(reg, vals) = inst {
                // Check if we already have this exact value
                let idx = immediates.iter().position(|v: &[f32; 4]| {
                    v[0] == vals[0] && v[1] == vals[1] && v[2] == vals[2] && v[3] == vals[3]
                });
                let idx = match idx {
                    Some(i) => i as u32,
                    None => {
                        let i = immediates.len() as u32;
                        immediates.push(*vals);
                        i
                    }
                };
                if (*reg as usize) < imm_map.len() {
                    imm_map[*reg as usize] = Some(idx);
                }
            }
        }

        // Remove registers from imm_map if they are written by any instruction
        // other than LoadConst (e.g. WriteMask modifies the register in-place,
        // so it can no longer be referenced as an IMM literal).
        for inst in &prog.instructions {
            let dst = match inst {
                Inst::LoadConst(_, _) => continue,
                Inst::StorePosition(_) | Inst::StoreFragColor(_)
                | Inst::StorePointSize(_) | Inst::StoreVarying(_, _)
                | Inst::Discard => continue,
                Inst::Mov(d, _) | Inst::Neg(d, _) | Inst::Abs(d, _)
                | Inst::Floor(d, _) | Inst::Fract(d, _) | Inst::Sqrt(d, _)
                | Inst::Rsqrt(d, _) | Inst::Sin(d, _) | Inst::Cos(d, _)
                | Inst::Normalize(d, _) | Inst::Length(d, _)
                | Inst::IntToFloat(d, _) | Inst::FloatToInt(d, _)
                | Inst::LoadVarying(d, _) | Inst::LoadUniform(d, _)
                | Inst::LoadAttribute(d, _) => *d,
                Inst::Add(d, _, _) | Inst::Sub(d, _, _) | Inst::Mul(d, _, _)
                | Inst::Div(d, _, _) | Inst::Dp3(d, _, _) | Inst::Dp4(d, _, _)
                | Inst::Cross(d, _, _) | Inst::Min(d, _, _) | Inst::Max(d, _, _)
                | Inst::Pow(d, _, _) | Inst::Reflect(d, _, _)
                | Inst::TexSample(d, _, _) | Inst::MatMul4(d, _, _)
                | Inst::MatMul3(d, _, _) | Inst::CmpLt(d, _, _)
                | Inst::CmpEq(d, _, _) => *d,
                Inst::Swizzle(d, _, _, _) | Inst::WriteMask(d, _, _) => *d,
                Inst::Clamp(d, _, _, _) | Inst::Mix(d, _, _, _)
                | Inst::Select(d, _, _, _) => *d,
            };
            if (dst as usize) < imm_map.len() {
                imm_map[dst as usize] = None;
            }
        }

        // Find the actual max CONST index referenced by LoadUniform instructions
        let mut max_const_idx = 0u32;
        let mut has_const = false;
        for inst in &prog.instructions {
            if let Inst::LoadUniform(_, idx) = inst {
                has_const = true;
                if *idx >= max_const_idx {
                    max_const_idx = *idx;
                }
            }
        }
        let num_consts = if has_const { max_const_idx + 1 } else { 0 };

        // Build sampler register → unit mapping from TexSample instructions
        let mut sampler_map = Vec::new();
        sampler_map.resize(prog.num_regs as usize, 0u32);
        let mut next_samp = 0u32;
        for inst in &prog.instructions {
            if let Inst::TexSample(_, samp_reg, _) = inst {
                if sampler_map[*samp_reg as usize] == 0 && next_samp == 0 {
                    // First sampler gets unit 0
                    next_samp = 1;
                } else if sampler_map[*samp_reg as usize] == 0 {
                    sampler_map[*samp_reg as usize] = next_samp;
                    next_samp += 1;
                }
            }
        }

        Self {
            prog,
            is_vertex,
            num_samplers,
            out: String::with_capacity(4096),
            pc: 0,
            immediates,
            imm_map,
            // +1 extra TEMP for cross product scratch (avoids colliding with IR regs)
            num_temps: prog.num_regs + 1,
            num_consts,
            const_base,
            sampler_map,
        }
    }

    fn emit_header(&mut self) {
        if self.is_vertex {
            let _ = write!(self.out, "VERT\n");
        } else {
            let _ = write!(self.out, "FRAG\n");
        }
    }

    fn emit_declarations(&mut self) {
        if self.is_vertex {
            // Vertex inputs (attributes)
            for (i, _attr) in self.prog.attributes.iter().enumerate() {
                let _ = write!(self.out, "DCL IN[{}]\n", i);
            }
            // Outputs: position + varyings as GENERIC
            let _ = write!(self.out, "DCL OUT[0], POSITION\n");
            for (i, _v) in self.prog.varyings.iter().enumerate() {
                let _ = write!(self.out, "DCL OUT[{}], GENERIC[{}]\n", i + 1, i);
            }
        } else {
            // Fragment inputs (varyings from VS as GENERIC)
            for (i, _v) in self.prog.varyings.iter().enumerate() {
                let _ = write!(self.out, "DCL IN[{}], GENERIC[{}], PERSPECTIVE\n", i, i);
            }
            // Fragment output: COLOR
            let _ = write!(self.out, "DCL OUT[0], COLOR\n");
        }

        // Constant buffer (uniforms) — 2D syntax: CONST[buffer][range]
        // Declare the full range including the base offset so FS can access its
        // uniforms at const_base + local_slot within the shared combined buffer.
        if self.num_consts > 0 {
            let last_slot = self.const_base + self.num_consts - 1;
            if last_slot == 0 {
                let _ = write!(self.out, "DCL CONST[0][0]\n");
            } else {
                let _ = write!(self.out, "DCL CONST[0][0..{}]\n", last_slot);
            }
        }

        // Samplers
        for i in 0..self.num_samplers {
            let _ = write!(self.out, "DCL SAMP[{}]\n", i);
        }
        // Sampler views (for TEX instruction)
        for i in 0..self.num_samplers {
            let _ = write!(self.out, "DCL SVIEW[{}], 2D, FLOAT\n", i);
        }

        // Temporaries
        if self.num_temps > 0 {
            if self.num_temps == 1 {
                let _ = write!(self.out, "DCL TEMP[0]\n");
            } else {
                let _ = write!(self.out, "DCL TEMP[0..{}]\n", self.num_temps - 1);
            }
        }
    }

    fn emit_immediates(&mut self) {
        for imm in &self.immediates {
            let _ = write!(self.out, "IMM FLT32 {{ {}, {}, {}, {} }}\n",
                FmtFloat(imm[0]), FmtFloat(imm[1]),
                FmtFloat(imm[2]), FmtFloat(imm[3]));
        }
    }

    /// Format a source register reference.
    fn src(&self, reg: Reg) -> String {
        // Check if this register is an immediate
        if let Some(Some(imm_idx)) = self.imm_map.get(reg as usize) {
            let mut s = String::new();
            let _ = write!(s, "IMM[{}]", imm_idx);
            return s;
        }
        let mut s = String::new();
        let _ = write!(s, "TEMP[{}]", reg);
        s
    }

    /// Format a destination register reference.
    fn dst(&self, reg: Reg) -> String {
        let mut s = String::new();
        let _ = write!(s, "TEMP[{}]", reg);
        s
    }

    fn emit(&mut self, text: &str) {
        let _ = write!(self.out, "  {}: {}\n", self.pc, text);
        self.pc += 1;
    }

    fn emit_body(&mut self) {
        // Clone instructions to avoid borrow issues
        let instructions = self.prog.instructions.clone();
        for inst in &instructions {
            self.emit_inst(inst);
        }
    }

    fn emit_inst(&mut self, inst: &Inst) {
        match inst {
            Inst::LoadConst(_, _) => {
                // Handled via IMM declarations, no runtime instruction needed
            }

            Inst::Mov(d, s) => {
                let line = fmt2("MOV", &self.dst(*d), &self.src(*s));
                self.emit(&line);
            }

            Inst::Add(d, a, b) => {
                let line = fmt3("ADD", &self.dst(*d), &self.src(*a), &self.src(*b));
                self.emit(&line);
            }
            Inst::Sub(d, a, b) => {
                let line = fmt3("ADD", &self.dst(*d), &self.src(*a), &negate(&self.src(*b)));
                self.emit(&line);
            }
            Inst::Mul(d, a, b) => {
                let line = fmt3("MUL", &self.dst(*d), &self.src(*a), &self.src(*b));
                self.emit(&line);
            }
            Inst::Div(d, a, b) => {
                // DIV = RCP + MUL
                let tmp = self.dst(*d);
                let line = fmt2("RCP", &tmp, &self.src(*b));
                self.emit(&line);
                let line = fmt3("MUL", &tmp, &self.src(*a), &tmp);
                self.emit(&line);
            }
            Inst::Neg(d, s) => {
                let line = fmt2("MOV", &self.dst(*d), &negate(&self.src(*s)));
                self.emit(&line);
            }

            Inst::Dp3(d, a, b) => {
                let line = fmt3("DP3", &self.dst(*d), &self.src(*a), &self.src(*b));
                self.emit(&line);
            }
            Inst::Dp4(d, a, b) => {
                let line = fmt3("DP4", &self.dst(*d), &self.src(*a), &self.src(*b));
                self.emit(&line);
            }

            Inst::Cross(d, a, b) => {
                // Cross product: XPD was removed in later TGSI. Use MUL+SUB:
                // dst.xyz = a.yzx * b.zxy - a.zxy * b.yzx
                let dst = self.dst(*d);
                let sa = self.src(*a);
                let sb = self.src(*b);
                // Use the dedicated scratch TEMP (last register, beyond IR regs)
                let tmp_name = format!("TEMP[{}]", self.num_temps - 1);
                let line = fmt3("MUL", &dst, &swz(&sa, "yzxw"), &swz(&sb, "zxyw"));
                self.emit(&line);
                let line = fmt3("MUL", &tmp_name, &swz(&sa, "zxyw"), &swz(&sb, "yzxw"));
                self.emit(&line);
                let line = fmt3("ADD", &dst, &dst, &negate(&tmp_name));
                self.emit(&line);
            }

            Inst::Normalize(d, s) => {
                // normalize(v) = v * rsq(dp3(v,v))
                let dst = self.dst(*d);
                let src = self.src(*s);
                let line = fmt3("DP3", &dst, &src, &src);
                self.emit(&line);
                let line = fmt2("RSQ", &dst, &swz(&dst, "xxxx"));
                self.emit(&line);
                let line = fmt3("MUL", &dst, &src, &dst);
                self.emit(&line);
            }
            Inst::Length(d, s) => {
                // length(v) = sqrt(dp3(v,v))
                let dst = self.dst(*d);
                let src = self.src(*s);
                let line = fmt3("DP3", &dst, &src, &src);
                self.emit(&line);
                let line = fmt2("SQRT", &dst, &swz(&dst, "xxxx"));
                self.emit(&line);
            }

            Inst::Min(d, a, b) => {
                let line = fmt3("MIN", &self.dst(*d), &self.src(*a), &self.src(*b));
                self.emit(&line);
            }
            Inst::Max(d, a, b) => {
                let line = fmt3("MAX", &self.dst(*d), &self.src(*a), &self.src(*b));
                self.emit(&line);
            }
            Inst::Clamp(d, x, lo, hi) => {
                let dst = self.dst(*d);
                let line = fmt3("MAX", &dst, &self.src(*x), &self.src(*lo));
                self.emit(&line);
                let line = fmt3("MIN", &dst, &dst, &self.src(*hi));
                self.emit(&line);
            }
            Inst::Mix(d, a, b, t) => {
                // mix(a, b, t) = a + t * (b - a)  →  LRP dst, t, b, a
                // LRP dst, t, b, a = t*b + (1-t)*a = mix(a,b,t)
                let line = fmt4("LRP", &self.dst(*d), &self.src(*t), &self.src(*b), &self.src(*a));
                self.emit(&line);
            }

            Inst::Abs(d, s) => {
                let line = fmt2("MOV", &self.dst(*d), &abs(&self.src(*s)));
                self.emit(&line);
            }
            Inst::Floor(d, s) => {
                let line = fmt2("FLR", &self.dst(*d), &self.src(*s));
                self.emit(&line);
            }
            Inst::Fract(d, s) => {
                let line = fmt2("FRC", &self.dst(*d), &self.src(*s));
                self.emit(&line);
            }

            Inst::Pow(d, base, exp) => {
                let line = fmt3("POW", &self.dst(*d), &swz(&self.src(*base), "xxxx"), &swz(&self.src(*exp), "xxxx"));
                self.emit(&line);
            }
            Inst::Sqrt(d, s) => {
                let line = fmt2("SQRT", &self.dst(*d), &self.src(*s));
                self.emit(&line);
            }
            Inst::Rsqrt(d, s) => {
                let line = fmt2("RSQ", &self.dst(*d), &self.src(*s));
                self.emit(&line);
            }

            Inst::Sin(d, s) => {
                let line = fmt2("SIN", &self.dst(*d), &swz(&self.src(*s), "xxxx"));
                self.emit(&line);
            }
            Inst::Cos(d, s) => {
                let line = fmt2("COS", &self.dst(*d), &swz(&self.src(*s), "xxxx"));
                self.emit(&line);
            }

            Inst::Reflect(d, i, n) => {
                // reflect(I, N) = I - 2 * dot(N, I) * N
                let dst = self.dst(*d);
                let si = self.src(*i);
                let sn = self.src(*n);
                let line = fmt3("DP3", &dst, &sn, &si);
                self.emit(&line);
                let line = fmt3("MUL", &dst, &dst, &sn);
                self.emit(&line);
                let line = fmt3("ADD", &dst, &dst, &dst);
                self.emit(&line);
                let line = fmt3("ADD", &dst, &si, &negate(&dst));
                self.emit(&line);
            }

            Inst::TexSample(d, sampler, coord) => {
                // TEX dst, coord, SAMP[unit], 2D
                let dst = self.dst(*d);
                let coord_s = self.src(*coord);
                let unit = self.sampler_map.get(*sampler as usize).copied().unwrap_or(0);
                let mut line = String::new();
                let _ = write!(line, "TEX {}, {}, SAMP[{}], 2D", dst, coord_s, unit);
                self.emit(&line);
            }

            Inst::MatMul4(d, mat, vec) => {
                // Column-major mat4 * vec4:
                //   result = col0*v.x + col1*v.y + col2*v.z + col3*v.w
                // Matches JIT backend's emit_matmul4 convention.
                let dst = self.dst(*d);
                let sv = self.src(*vec);
                let col0 = format!("TEMP[{}]", mat);
                let col1 = format!("TEMP[{}]", mat + 1);
                let col2 = format!("TEMP[{}]", mat + 2);
                let col3 = format!("TEMP[{}]", mat + 3);
                self.emit(&fmt3("MUL", &dst, &col0, &swz(&sv, "xxxx")));
                self.emit(&fmt4("MAD", &dst, &col1, &swz(&sv, "yyyy"), &dst));
                self.emit(&fmt4("MAD", &dst, &col2, &swz(&sv, "zzzz"), &dst));
                self.emit(&fmt4("MAD", &dst, &col3, &swz(&sv, "wwww"), &dst));
            }
            Inst::MatMul3(d, mat, vec) => {
                // Column-major mat3 * vec3:
                //   result.xyz = col0*v.x + col1*v.y + col2*v.z
                let dst = self.dst(*d);
                let sv = self.src(*vec);
                let col0 = format!("TEMP[{}]", mat);
                let col1 = format!("TEMP[{}]", mat + 1);
                let col2 = format!("TEMP[{}]", mat + 2);
                self.emit(&fmt3("MUL", &dst, &col0, &swz(&sv, "xxxx")));
                self.emit(&fmt4("MAD", &dst, &col1, &swz(&sv, "yyyy"), &dst));
                self.emit(&fmt4("MAD", &dst, &col2, &swz(&sv, "zzzz"), &dst));
            }

            Inst::Swizzle(d, s, indices, _count) => {
                let comps = ["x", "y", "z", "w"];
                let swz_str = format!("{}{}{}{}",
                    comps[indices[0] as usize], comps[indices[1] as usize],
                    comps[indices[2] as usize], comps[indices[3] as usize]);
                let line = fmt2("MOV", &self.dst(*d), &swz(&self.src(*s), &swz_str));
                self.emit(&line);
            }

            Inst::WriteMask(d, s, mask) => {
                // Write specific components. TGSI uses write mask on dst.
                let comps = mask_to_str(*mask);
                let mut line = String::new();
                let _ = write!(line, "MOV {}.{}, {}", self.dst(*d), comps, self.src(*s));
                self.emit(&line);
            }

            Inst::CmpLt(d, a, b) => {
                // SLT dst, a, b  (dst = a < b ? 1.0 : 0.0)
                let line = fmt3("SLT", &self.dst(*d), &self.src(*a), &self.src(*b));
                self.emit(&line);
            }
            Inst::CmpEq(d, a, b) => {
                // SEQ dst, a, b
                let line = fmt3("SEQ", &self.dst(*d), &self.src(*a), &self.src(*b));
                self.emit(&line);
            }

            Inst::Select(d, cond, a, b) => {
                // CMP dst, -cond, a, b  (if cond < 0 then a else b)
                // But our Select is: cond != 0 ? a : b
                // Use: CMP dst, -cond, b, a → if (-cond < 0 i.e. cond > 0) → b... no.
                // TGSI CMP: dst = (src0 < 0) ? src1 : src2
                // We want: cond != 0 → a, else b
                // So: negate cond, CMP dst, -cond, a, b → if cond > 0 → a, if cond <= 0 → b
                // Close enough for typical use (cond is 0.0 or 1.0)
                let line = fmt4("CMP", &self.dst(*d), &negate(&self.src(*cond)), &self.src(*a), &self.src(*b));
                self.emit(&line);
            }

            Inst::IntToFloat(d, s) | Inst::FloatToInt(d, s) => {
                // TGSI: I2F / F2I, but for simplicity just MOV (GLES2 doesn't have real ints)
                let line = fmt2("MOV", &self.dst(*d), &self.src(*s));
                self.emit(&line);
            }

            Inst::StorePosition(s) => {
                let line = fmt2("MOV", "OUT[0]", &self.src(*s));
                self.emit(&line);
            }
            Inst::StoreFragColor(s) => {
                let line = fmt2("MOV", "OUT[0]", &self.src(*s));
                self.emit(&line);
            }
            Inst::StorePointSize(s) => {
                // Point size output — skip for now, most virgl contexts don't need it
                let _ = s;
            }

            Inst::LoadVarying(d, idx) => {
                if self.is_vertex {
                    // VS doesn't load varyings, they're outputs
                    let line = fmt2("MOV", &self.dst(*d), "TEMP[0]");
                    self.emit(&line);
                } else {
                    // FS: varyings are IN[idx]
                    let mut line = String::new();
                    let _ = write!(line, "MOV {}, IN[{}]", self.dst(*d), idx);
                    self.emit(&line);
                }
            }
            Inst::StoreVarying(idx, s) => {
                if self.is_vertex {
                    // VS: varyings are OUT[idx+1] (OUT[0] is POSITION)
                    let mut line = String::new();
                    let _ = write!(line, "MOV OUT[{}], {}", idx + 1, self.src(*s));
                    self.emit(&line);
                }
                // FS doesn't store varyings
            }

            Inst::LoadUniform(d, idx) => {
                let mut line = String::new();
                let abs_slot = self.const_base + idx;
                let _ = write!(line, "MOV {}, CONST[0][{}]", self.dst(*d), abs_slot);
                self.emit(&line);
            }

            Inst::LoadAttribute(d, idx) => {
                if self.is_vertex {
                    let mut line = String::new();
                    let _ = write!(line, "MOV {}, IN[{}]", self.dst(*d), idx);
                    self.emit(&line);
                }
            }

            Inst::Discard => {
                self.emit("KILL");
            }
        }
    }

    fn emit_end(&mut self) {
        let _ = write!(self.out, "  {}: END\n", self.pc);
    }
}

// ── Formatting helpers ────────────────────────────────────────────────────

fn fmt2(op: &str, dst: &str, src: &str) -> String {
    let mut s = String::new();
    let _ = write!(s, "{} {}, {}", op, dst, src);
    s
}

fn fmt3(op: &str, dst: &str, a: &str, b: &str) -> String {
    let mut s = String::new();
    let _ = write!(s, "{} {}, {}, {}", op, dst, a, b);
    s
}

fn fmt4(op: &str, dst: &str, a: &str, b: &str, c: &str) -> String {
    let mut s = String::new();
    let _ = write!(s, "{} {}, {}, {}, {}", op, dst, a, b, c);
    s
}

fn swz(reg: &str, components: &str) -> String {
    let mut s = String::new();
    let _ = write!(s, "{}.{}", reg, components);
    s
}

fn negate(reg: &str) -> String {
    let mut s = String::new();
    let _ = write!(s, "-{}", reg);
    s
}

fn abs(reg: &str) -> String {
    let mut s = String::new();
    let _ = write!(s, "|{}|", reg);
    s
}

fn mask_to_str(mask: u8) -> String {
    let mut s = String::new();
    if mask & 1 != 0 { s.push('x'); }
    if mask & 2 != 0 { s.push('y'); }
    if mask & 4 != 0 { s.push('z'); }
    if mask & 8 != 0 { s.push('w'); }
    if s.is_empty() { s.push_str("xyzw"); }
    s
}

/// Format a float for TGSI (always with decimal point).
struct FmtFloat(f32);

impl core::fmt::Display for FmtFloat {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        if self.0 == 0.0 {
            write!(f, "0.0")
        } else if self.0 == 1.0 {
            write!(f, "1.0")
        } else if self.0 == -1.0 {
            write!(f, "-1.0")
        } else if self.0 == (self.0 as i32) as f32 {
            write!(f, "{}.0", self.0 as i32)
        } else {
            write!(f, "{}", self.0)
        }
    }
}
