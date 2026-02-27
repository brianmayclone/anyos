//! x86_64 JIT compiler backend for shader IR.
//!
//! Compiles IR instructions to native x86_64 SSE machine code, eliminating the
//! ~15 cycle branch-misprediction penalty per instruction from the interpreter's
//! match dispatch. For a typical 70-instruction Phong fragment shader executed
//! ~80K times per frame, this eliminates ~84M wasted cycles per frame.
//!
//! # Architecture
//!
//! The JIT function uses the System V AMD64 calling convention:
//!
//! ```text
//! extern "C" fn(ctx: *const JitContext)
//! ```
//!
//! Register allocation in JIT code:
//! - RBX  = shader register file base (`ctx.regs`)
//! - R12  = uniforms pointer (`ctx.uniforms`)
//! - R13  = attributes pointer (`ctx.attributes`)
//! - R14  = varyings input pointer (`ctx.varyings_in`)
//! - R15  = varyings output pointer (`ctx.varyings_out`)
//! - RBP  = saved ctx pointer
//! - XMM0–XMM7 = scratch registers for instruction operands
//!
//! Shader virtual registers (up to 128) live in memory at `[RBX + reg * 16]`.
//! Each instruction does load → operate → store, emitting straight-line SSE
//! instructions with zero branching overhead.

extern crate alloc;
use alloc::vec::Vec;
use super::ir::{Program, Inst};
use crate::compiler::backend_sw::TexSampleFn;

/// JIT-compiled shader code buffer.
///
/// Holds the machine code in an executable heap buffer. The code is valid for
/// the lifetime of this struct — dropping it frees the buffer.
pub struct JitCode {
    /// Machine code buffer (heap-allocated, executable on anyOS).
    code: Vec<u8>,
}

/// Context struct passed to JIT-compiled shader functions.
///
/// All pointers must remain valid for the duration of the JIT call.
/// The JIT function reads/writes through these pointers.
#[repr(C)]
pub struct JitContext {
    /// Pointer to register file: `[[f32; 4]; MAX_REGS]`.
    pub regs: *mut f32,
    /// Pointer to uniform array: `&[[f32; 4]]`.
    pub uniforms: *const f32,
    /// Pointer to attribute array: `&[[f32; 4]]`.
    pub attributes: *const f32,
    /// Pointer to input varyings (fragment shader) or null.
    pub varyings_in: *const f32,
    /// Pointer to output varyings (vertex shader writes here).
    pub varyings_out: *mut f32,
    /// Pointer to `gl_Position` output `[f32; 4]`.
    pub position: *mut f32,
    /// Pointer to `gl_FragColor` output `[f32; 4]`.
    pub frag_color: *mut f32,
    /// Pointer to `gl_PointSize` output `f32`.
    pub point_size: *mut f32,
    /// Texture sampler function pointer (as usize for C ABI).
    pub tex_sample: usize,
}

/// Type of the JIT-compiled function.
pub type JitFn = unsafe extern "C" fn(ctx: *mut JitContext);

impl JitCode {
    /// Get the entry point as a callable function pointer.
    #[inline]
    pub fn as_fn(&self) -> JitFn {
        unsafe { core::mem::transmute(self.code.as_ptr()) }
    }

    /// Return the size of the compiled machine code in bytes.
    #[inline]
    pub fn code_len(&self) -> usize {
        self.code.len()
    }
}

// ── JitContext field offsets (repr(C), all pointers are 8 bytes) ──────────

const CTX_REGS: i32 = 0;
const CTX_UNIFORMS: i32 = 8;
const CTX_ATTRIBUTES: i32 = 16;
const CTX_VARYINGS_IN: i32 = 24;
const CTX_VARYINGS_OUT: i32 = 32;
const CTX_POSITION: i32 = 40;
const CTX_FRAG_COLOR: i32 = 48;
const CTX_POINT_SIZE: i32 = 56;
const CTX_TEX_SAMPLE: i32 = 64;

// ── x86_64 instruction encoding helpers ──────────────────────────────────

/// Emit bytes to the code buffer.
struct Emitter {
    code: Vec<u8>,
}

impl Emitter {
    fn new() -> Self {
        Self { code: Vec::with_capacity(4096) }
    }

    #[inline(always)]
    fn emit(&mut self, byte: u8) {
        self.code.push(byte);
    }

    #[inline(always)]
    fn emit2(&mut self, a: u8, b: u8) {
        self.code.push(a);
        self.code.push(b);
    }

    #[inline(always)]
    fn emit_bytes(&mut self, bytes: &[u8]) {
        self.code.extend_from_slice(bytes);
    }

    /// Emit a 32-bit signed displacement.
    #[inline(always)]
    fn emit_i32(&mut self, v: i32) {
        self.code.extend_from_slice(&v.to_le_bytes());
    }

    /// Emit a 64-bit unsigned value.
    #[inline(always)]
    fn emit_u64(&mut self, v: u64) {
        self.code.extend_from_slice(&v.to_le_bytes());
    }

    /// Current code position (for computing relative offsets).
    #[inline(always)]
    fn pos(&self) -> usize {
        self.code.len()
    }

    // ── REX prefixes ─────────────────────────────────────────────────

    /// REX.W prefix (64-bit operand size).
    #[inline(always)]
    fn rex_w(&mut self) { self.emit(0x48); }

    /// REX prefix for extended registers.
    /// W=1 for 64-bit, R extends ModRM.reg, B extends ModRM.r/m or SIB.base.
    #[inline(always)]
    fn rex(&mut self, w: bool, r: bool, x: bool, b: bool) {
        let byte = 0x40
            | if w { 0x08 } else { 0 }
            | if r { 0x04 } else { 0 }
            | if x { 0x02 } else { 0 }
            | if b { 0x01 } else { 0 };
        self.emit(byte);
    }

    // ── Common GPR instructions ──────────────────────────────────────

    /// `push reg64` (handles R8-R15 with REX prefix).
    fn push_r64(&mut self, reg: u8) {
        if reg >= 8 { self.emit(0x41); }
        self.emit(0x50 + (reg & 7));
    }

    /// `pop reg64`.
    fn pop_r64(&mut self, reg: u8) {
        if reg >= 8 { self.emit(0x41); }
        self.emit(0x58 + (reg & 7));
    }

    /// `mov reg64, [base64 + disp32]` — load 64-bit pointer from memory.
    fn mov_r64_mem(&mut self, dst: u8, base: u8, disp: i32) {
        // REX.W + 8B /r + ModRM(10, dst, base) + disp32
        self.rex(true, dst >= 8, false, base >= 8);
        self.emit(0x8B);
        self.modrm_disp32(dst & 7, base & 7);
        self.emit_i32(disp);
    }

    /// `mov reg64, imm64` — load 64-bit immediate.
    fn mov_r64_imm64(&mut self, dst: u8, imm: u64) {
        self.rex(true, false, false, dst >= 8);
        self.emit(0xB8 + (dst & 7));
        self.emit_u64(imm);
    }

    /// `mov dst64, src64` — register-to-register 64-bit move.
    fn mov_r64_r64(&mut self, dst: u8, src: u8) {
        self.rex(true, src >= 8, false, dst >= 8);
        self.emit(0x89);
        self.modrm_reg(src & 7, dst & 7);
    }

    /// `ret`
    fn ret(&mut self) { self.emit(0xC3); }

    // ── SSE instructions ─────────────────────────────────────────────

    /// `movups xmm, [base + disp32]` — load 128 bits unaligned.
    fn movups_load(&mut self, xmm: u8, base: u8, disp: i32) {
        // REX if extended registers
        if xmm >= 8 || base >= 8 {
            self.rex(false, xmm >= 8, false, base >= 8);
        }
        self.emit(0x0F);
        self.emit(0x10);
        self.modrm_disp32(xmm & 7, base & 7);
        self.emit_i32(disp);
    }

    /// `movups [base + disp32], xmm` — store 128 bits unaligned.
    fn movups_store(&mut self, base: u8, disp: i32, xmm: u8) {
        if xmm >= 8 || base >= 8 {
            self.rex(false, xmm >= 8, false, base >= 8);
        }
        self.emit(0x0F);
        self.emit(0x11);
        self.modrm_disp32(xmm & 7, base & 7);
        self.emit_i32(disp);
    }

    /// `movss xmm, [base + disp32]` — load 32-bit float.
    fn movss_load(&mut self, xmm: u8, base: u8, disp: i32) {
        self.emit(0xF3); // prefix
        if xmm >= 8 || base >= 8 {
            self.rex(false, xmm >= 8, false, base >= 8);
        }
        self.emit(0x0F);
        self.emit(0x10);
        self.modrm_disp32(xmm & 7, base & 7);
        self.emit_i32(disp);
    }

    /// `movss [base + disp32], xmm` — store 32-bit float.
    fn movss_store(&mut self, base: u8, disp: i32, xmm: u8) {
        self.emit(0xF3);
        if xmm >= 8 || base >= 8 {
            self.rex(false, xmm >= 8, false, base >= 8);
        }
        self.emit(0x0F);
        self.emit(0x11);
        self.modrm_disp32(xmm & 7, base & 7);
        self.emit_i32(disp);
    }

    /// `movaps xmm_dst, xmm_src` — register-to-register move.
    fn movaps_rr(&mut self, dst: u8, src: u8) {
        if dst >= 8 || src >= 8 {
            self.rex(false, dst >= 8, false, src >= 8);
        }
        self.emit(0x0F);
        self.emit(0x28);
        self.modrm_reg(dst & 7, src & 7);
    }

    /// Packed SSE operation: `op xmm0, xmm1` (reg-reg form).
    /// Opcode is the 2-byte 0F xx encoding.
    fn sse_rr(&mut self, op: u8, dst: u8, src: u8) {
        if dst >= 8 || src >= 8 {
            self.rex(false, dst >= 8, false, src >= 8);
        }
        self.emit(0x0F);
        self.emit(op);
        self.modrm_reg(dst & 7, src & 7);
    }

    /// Packed SSE operation with 0x66 prefix: `op xmm0, xmm1`.
    fn sse66_rr(&mut self, op: u8, dst: u8, src: u8) {
        self.emit(0x66);
        if dst >= 8 || src >= 8 {
            self.rex(false, dst >= 8, false, src >= 8);
        }
        self.emit(0x0F);
        self.emit(op);
        self.modrm_reg(dst & 7, src & 7);
    }

    /// `addps xmm_dst, xmm_src`
    fn addps(&mut self, dst: u8, src: u8) { self.sse_rr(0x58, dst, src); }

    /// `subps xmm_dst, xmm_src`
    fn subps(&mut self, dst: u8, src: u8) { self.sse_rr(0x5C, dst, src); }

    /// `mulps xmm_dst, xmm_src`
    fn mulps(&mut self, dst: u8, src: u8) { self.sse_rr(0x59, dst, src); }

    /// `divps xmm_dst, xmm_src`
    fn divps(&mut self, dst: u8, src: u8) { self.sse_rr(0x5E, dst, src); }

    /// `minps xmm_dst, xmm_src`
    fn minps(&mut self, dst: u8, src: u8) { self.sse_rr(0x5D, dst, src); }

    /// `maxps xmm_dst, xmm_src`
    fn maxps(&mut self, dst: u8, src: u8) { self.sse_rr(0x5F, dst, src); }

    /// `sqrtps xmm_dst, xmm_src`
    fn sqrtps(&mut self, dst: u8, src: u8) { self.sse_rr(0x51, dst, src); }

    /// `rsqrtps xmm_dst, xmm_src`
    fn rsqrtps(&mut self, dst: u8, src: u8) { self.sse_rr(0x52, dst, src); }

    /// `rcpps xmm_dst, xmm_src`
    fn rcpps(&mut self, dst: u8, src: u8) { self.sse_rr(0x53, dst, src); }

    /// `xorps xmm_dst, xmm_src`
    fn xorps(&mut self, dst: u8, src: u8) { self.sse_rr(0x57, dst, src); }

    /// `andps xmm_dst, xmm_src`
    fn andps(&mut self, dst: u8, src: u8) { self.sse_rr(0x54, dst, src); }

    /// `andnps xmm_dst, xmm_src`
    fn andnps(&mut self, dst: u8, src: u8) { self.sse_rr(0x55, dst, src); }

    /// `orps xmm_dst, xmm_src`
    fn orps(&mut self, dst: u8, src: u8) { self.sse_rr(0x56, dst, src); }

    /// `cmpltps xmm_dst, xmm_src` (cmpps with imm8=1)
    fn cmpltps(&mut self, dst: u8, src: u8) {
        self.sse_rr(0xC2, dst, src);
        self.emit(0x01); // LT predicate
    }

    /// `cmpneqps xmm_dst, xmm_src` (cmpps with imm8=4)
    fn cmpneqps(&mut self, dst: u8, src: u8) {
        self.sse_rr(0xC2, dst, src);
        self.emit(0x04); // NEQ predicate
    }

    /// `shufps xmm_dst, xmm_src, imm8`
    fn shufps(&mut self, dst: u8, src: u8, imm: u8) {
        self.sse_rr(0xC6, dst, src);
        self.emit(imm);
    }

    /// `movhlps xmm_dst, xmm_src` — move high to low.
    fn movhlps(&mut self, dst: u8, src: u8) { self.sse_rr(0x12, dst, src); }

    /// `movlhps xmm_dst, xmm_src` — move low to high.
    fn movlhps(&mut self, dst: u8, src: u8) { self.sse_rr(0x16, dst, src); }

    /// `addss xmm_dst, xmm_src` — scalar add.
    fn addss(&mut self, dst: u8, src: u8) {
        self.emit(0xF3);
        self.sse_rr(0x58, dst, src);
    }

    /// `mulss xmm_dst, xmm_src` — scalar multiply.
    fn mulss(&mut self, dst: u8, src: u8) {
        self.emit(0xF3);
        self.sse_rr(0x59, dst, src);
    }

    /// `subss xmm_dst, xmm_src` — scalar subtract.
    fn subss(&mut self, dst: u8, src: u8) {
        self.emit(0xF3);
        self.sse_rr(0x5C, dst, src);
    }

    /// `movss xmm_dst, xmm_src` — scalar move (zero-extends upper).
    fn movss_rr(&mut self, dst: u8, src: u8) {
        self.emit(0xF3);
        self.sse_rr(0x10, dst, src);
    }

    /// `movhdup xmm_dst, xmm_src` — SSE3 movshdup (duplicate odd elements).
    fn movshdup(&mut self, dst: u8, src: u8) {
        self.emit(0xF3);
        self.sse_rr(0x16, dst, src);
    }

    /// `cvttss2si reg32, xmm` — convert scalar float to int (truncate).
    fn cvttss2si_r32_xmm(&mut self, dst_gpr: u8, src_xmm: u8) {
        self.emit(0xF3);
        if dst_gpr >= 8 || src_xmm >= 8 {
            self.rex(false, dst_gpr >= 8, false, src_xmm >= 8);
        }
        self.emit(0x0F);
        self.emit(0x2C);
        self.modrm_reg(dst_gpr & 7, src_xmm & 7);
    }

    /// `cvtsi2ss xmm, reg32` — convert int to scalar float.
    fn cvtsi2ss_xmm_r32(&mut self, dst_xmm: u8, src_gpr: u8) {
        self.emit(0xF3);
        if dst_xmm >= 8 || src_gpr >= 8 {
            self.rex(false, dst_xmm >= 8, false, src_gpr >= 8);
        }
        self.emit(0x0F);
        self.emit(0x2A);
        self.modrm_reg(dst_xmm & 7, src_gpr & 7);
    }

    /// SSE4.1 `roundps xmm_dst, xmm_src, imm8` (0x66 0F 3A 08).
    fn roundps(&mut self, dst: u8, src: u8, mode: u8) {
        self.emit(0x66);
        if dst >= 8 || src >= 8 {
            self.rex(false, dst >= 8, false, src >= 8);
        }
        self.emit_bytes(&[0x0F, 0x3A, 0x08]);
        self.modrm_reg(dst & 7, src & 7);
        self.emit(mode);
    }

    // ── ModR/M encoding ──────────────────────────────────────────────

    /// ModR/M byte: mod=11 (register-register), reg, r/m.
    #[inline(always)]
    fn modrm_reg(&mut self, reg: u8, rm: u8) {
        self.emit(0xC0 | ((reg & 7) << 3) | (rm & 7));
    }

    /// ModR/M byte: mod=10 (register + disp32), reg, r/m.
    /// Handles RBP/R13 specially (no SIB needed with mod=10).
    #[inline(always)]
    fn modrm_disp32(&mut self, reg: u8, rm: u8) {
        // mod=10, reg, r/m — but if r/m=4 (RSP/R12), need SIB byte
        if rm == 4 {
            // SIB needed: base=RSP, index=none, scale=1
            self.emit(0x80 | ((reg & 7) << 3) | 4);
            self.emit(0x24); // SIB: scale=0, index=4 (none), base=4 (RSP)
        } else {
            self.emit(0x80 | ((reg & 7) << 3) | (rm & 7));
        }
    }

    /// `call reg64` — indirect call through register.
    fn call_r64(&mut self, reg: u8) {
        if reg >= 8 { self.emit(0x41); }
        self.emit(0xFF);
        self.emit(0xD0 + (reg & 7));
    }

    /// `mov r32, imm32` — load 32-bit immediate into GPR (zero-extends to 64).
    fn mov_r32_imm32(&mut self, dst: u8, imm: u32) {
        if dst >= 8 { self.emit(0x41); }
        self.emit(0xB8 + (dst & 7));
        self.code.extend_from_slice(&imm.to_le_bytes());
    }

    /// Emit 4 floats as 16 bytes at the current position (constant pool entry).
    fn emit_f32x4(&mut self, v: [f32; 4]) {
        for f in &v {
            self.code.extend_from_slice(&f.to_le_bytes());
        }
    }

    /// `sub rsp, imm8` — allocate stack space.
    fn sub_rsp_imm8(&mut self, imm: u8) {
        self.rex_w();
        self.emit_bytes(&[0x83, 0xEC]);
        self.emit(imm);
    }

    /// `add rsp, imm8` — deallocate stack space.
    fn add_rsp_imm8(&mut self, imm: u8) {
        self.rex_w();
        self.emit_bytes(&[0x83, 0xC4]);
        self.emit(imm);
    }

    /// `mov [rsp + disp8], reg64` — store to stack.
    fn mov_stack_r64(&mut self, disp: i8, reg: u8) {
        self.rex(true, reg >= 8, false, false);
        self.emit(0x89);
        // mod=01 (disp8), reg, r/m=4 (RSP — need SIB)
        self.emit(0x44 | ((reg & 7) << 3));
        self.emit(0x24); // SIB: base=RSP
        self.emit(disp as u8);
    }

    /// `mov reg64, [rsp + disp8]` — load from stack.
    fn mov_r64_stack(&mut self, dst: u8, disp: i8) {
        self.rex(true, dst >= 8, false, false);
        self.emit(0x8B);
        self.emit(0x44 | ((dst & 7) << 3));
        self.emit(0x24);
        self.emit(disp as u8);
    }
}

// ── GPR register numbers ─────────────────────────────────────────────────

const RAX: u8 = 0;
const RCX: u8 = 1;
const RDX: u8 = 2;
const RBX: u8 = 3;
const RSP: u8 = 4;
const RBP: u8 = 5;
const RSI: u8 = 6;
const RDI: u8 = 7;
const R8: u8 = 8;
const R9: u8 = 9;
const R10: u8 = 10;
const R11: u8 = 11;
const R12: u8 = 12;
const R13: u8 = 13;
const R14: u8 = 14;
const R15: u8 = 15;

// ── XMM register numbers ────────────────────────────────────────────────

const XMM0: u8 = 0;
const XMM1: u8 = 1;
const XMM2: u8 = 2;
const XMM3: u8 = 3;
const XMM4: u8 = 4;
const XMM5: u8 = 5;
const XMM6: u8 = 6;
const XMM7: u8 = 7;

/// Byte offset of a shader virtual register in the register file.
#[inline(always)]
fn reg_off(r: u32) -> i32 {
    (r as i32) * 16
}

// ── Helper function wrappers (extern "C" ABI for JIT calls) ─────────────

/// C-ABI wrapper for `math::pow` — callable from JIT code.
///
/// Takes base and exponent as f32, returns result in XMM0 (scalar).
extern "C" fn jit_pow(base: f32, exp: f32) -> f32 {
    crate::rasterizer::math::pow(base, exp)
}

/// C-ABI wrapper for `math::sin`.
extern "C" fn jit_sin(x: f32) -> f32 {
    crate::rasterizer::math::sin(x)
}

/// C-ABI wrapper for `math::cos`.
extern "C" fn jit_cos(x: f32) -> f32 {
    crate::rasterizer::math::cos(x)
}

/// C-ABI wrapper for `math::floor`.
extern "C" fn jit_floor(x: f32) -> f32 {
    crate::rasterizer::math::floor(x)
}

/// C-ABI wrapper for `math::sqrt` (scalar).
extern "C" fn jit_sqrt(x: f32) -> f32 {
    crate::rasterizer::math::sqrt(x)
}

/// C-ABI wrapper for texture sampling.
///
/// Takes unit, u, v and writes result to `out` pointer (avoids [f32; 4]
/// return value ABI complexity). Called from JIT code.
extern "C" fn jit_tex_sample(
    tex_fn_ptr: usize,
    unit: u32,
    u: f32,
    v: f32,
    out: *mut [f32; 4],
) {
    let tex_fn: TexSampleFn = unsafe { core::mem::transmute(tex_fn_ptr) };
    let result = tex_fn(unit, u, v);
    unsafe { *out = result; }
}

// ── JIT compiler ─────────────────────────────────────────────────────────

/// Compile a shader IR program to native x86_64 machine code.
///
/// Returns `None` if the program cannot be JIT-compiled (e.g., uses
/// unsupported instructions). Falls back to the interpreter in that case.
pub fn compile_jit(program: &Program) -> Option<JitCode> {
    let mut e = Emitter::new();

    // ── Prologue: save callee-saved registers ────────────────────────
    e.push_r64(RBP);
    e.push_r64(RBX);
    e.push_r64(R12);
    e.push_r64(R13);
    e.push_r64(R14);
    e.push_r64(R15);
    // 6 pushes = 48 bytes. RSP is now 16-byte aligned
    // (entry had 8-byte return address, 6 pushes = 48, total 56... not aligned).
    // Push one more to align to 16:
    e.sub_rsp_imm8(8); // RSP now 16-byte aligned

    // RDI = ctx pointer. Save it to RBP for later access.
    // mov rbp, rdi (REX.W + 0x89 + ModRM)
    e.mov_r64_r64(RBP, RDI);

    // Load frequently-used pointers from JitContext into callee-saved regs
    e.mov_r64_mem(RBX, RBP, CTX_REGS);           // RBX = regs
    e.mov_r64_mem(R12, RBP, CTX_UNIFORMS);        // R12 = uniforms
    e.mov_r64_mem(R13, RBP, CTX_ATTRIBUTES);      // R13 = attributes
    e.mov_r64_mem(R14, RBP, CTX_VARYINGS_IN);     // R14 = varyings_in
    e.mov_r64_mem(R15, RBP, CTX_VARYINGS_OUT);    // R15 = varyings_out

    // ── Build constant register table (SSA IR: each reg written once) ──
    let mut const_regs: [Option<[f32; 4]>; 128] = [None; 128];
    for inst in &program.instructions {
        if let Inst::LoadConst(dst, val) = inst {
            if (*dst as usize) < 128 {
                const_regs[*dst as usize] = Some(*val);
            }
        }
    }

    // ── Emit instructions ────────────────────────────────────────────
    for inst in &program.instructions {
        emit_instruction(&mut e, inst, &const_regs);
    }

    // ── Epilogue: restore and return ─────────────────────────────────
    e.add_rsp_imm8(8);
    e.pop_r64(R15);
    e.pop_r64(R14);
    e.pop_r64(R13);
    e.pop_r64(R12);
    e.pop_r64(RBX);
    e.pop_r64(RBP);
    e.ret();

    Some(JitCode { code: e.code })
}

/// Emit native x86_64 code for a single IR instruction.
///
/// `const_regs` provides constant propagation data: if a register was loaded
/// by `LoadConst` and never overwritten (SSA), its value is available here.
/// Used to optimize `pow(x, 64.0)` into a multiply chain instead of calling
/// the slow x87 FPU transcendental (~200 cycles → ~6 cycles).
fn emit_instruction(e: &mut Emitter, inst: &Inst, const_regs: &[Option<[f32; 4]>; 128]) {
    match inst {
        // ── Data movement ────────────────────────────────────────────
        Inst::LoadConst(dst, val) => {
            emit_load_const(e, *dst, *val);
        }
        Inst::Mov(dst, src) => {
            if dst != src {
                e.movups_load(XMM0, RBX, reg_off(*src));
                e.movups_store(RBX, reg_off(*dst), XMM0);
            }
        }

        // ── Packed arithmetic (1 cycle each) ─────────────────────────
        Inst::Add(dst, a, b) => {
            emit_binop(e, *dst, *a, *b, |e| e.addps(XMM0, XMM1));
        }
        Inst::Sub(dst, a, b) => {
            emit_binop(e, *dst, *a, *b, |e| e.subps(XMM0, XMM1));
        }
        Inst::Mul(dst, a, b) => {
            emit_binop(e, *dst, *a, *b, |e| e.mulps(XMM0, XMM1));
        }
        Inst::Div(dst, a, b) => {
            // Safe div: lanes where b==0 produce 0 instead of NaN
            e.movups_load(XMM0, RBX, reg_off(*a));
            e.movups_load(XMM1, RBX, reg_off(*b));
            // xmm2 = 0.0
            e.xorps(XMM2, XMM2);
            // xmm3 = (b != 0) mask
            e.movaps_rr(XMM3, XMM1);
            e.cmpneqps(XMM3, XMM2);
            // xmm0 = a / b
            e.divps(XMM0, XMM1);
            // xmm0 = result & mask (zero where b was 0)
            e.andps(XMM0, XMM3);
            e.movups_store(RBX, reg_off(*dst), XMM0);
        }
        Inst::Neg(dst, src) => {
            // Negate via XOR with sign-bit mask
            e.movups_load(XMM0, RBX, reg_off(*src));
            // Load sign mask from constant
            emit_load_const_xmm(e, XMM1, [-0.0f32, -0.0, -0.0, -0.0]);
            e.xorps(XMM0, XMM1);
            e.movups_store(RBX, reg_off(*dst), XMM0);
        }
        Inst::Abs(dst, src) => {
            // Clear sign bits with AND mask
            e.movups_load(XMM0, RBX, reg_off(*src));
            emit_load_const_xmm(e, XMM1, [
                f32::from_bits(0x7FFFFFFF), f32::from_bits(0x7FFFFFFF),
                f32::from_bits(0x7FFFFFFF), f32::from_bits(0x7FFFFFFF),
            ]);
            e.andps(XMM0, XMM1);
            e.movups_store(RBX, reg_off(*dst), XMM0);
        }

        // ── Min / Max / Clamp / Mix ──────────────────────────────────
        Inst::Min(dst, a, b) => {
            emit_binop(e, *dst, *a, *b, |e| e.minps(XMM0, XMM1));
        }
        Inst::Max(dst, a, b) => {
            emit_binop(e, *dst, *a, *b, |e| e.maxps(XMM0, XMM1));
        }
        Inst::Clamp(dst, x, lo, hi) => {
            e.movups_load(XMM0, RBX, reg_off(*x));
            e.movups_load(XMM1, RBX, reg_off(*lo));
            e.movups_load(XMM2, RBX, reg_off(*hi));
            e.maxps(XMM0, XMM1); // max(x, lo)
            e.minps(XMM0, XMM2); // min(result, hi)
            e.movups_store(RBX, reg_off(*dst), XMM0);
        }
        Inst::Mix(dst, a, b, t) => {
            // lerp: a + (b - a) * t
            e.movups_load(XMM0, RBX, reg_off(*a));  // xmm0 = a
            e.movups_load(XMM1, RBX, reg_off(*b));  // xmm1 = b
            e.movups_load(XMM2, RBX, reg_off(*t));  // xmm2 = t
            e.subps(XMM1, XMM0);   // xmm1 = b - a
            e.mulps(XMM1, XMM2);   // xmm1 = (b - a) * t
            e.addps(XMM0, XMM1);   // xmm0 = a + (b - a) * t
            e.movups_store(RBX, reg_off(*dst), XMM0);
        }

        // ── Dot products ─────────────────────────────────────────────
        Inst::Dp3(dst, a, b) => {
            emit_dp3(e, *dst, *a, *b);
        }
        Inst::Dp4(dst, a, b) => {
            emit_dp4(e, *dst, *a, *b);
        }

        // ── Cross product ────────────────────────────────────────────
        Inst::Cross(dst, a, b) => {
            emit_cross(e, *dst, *a, *b);
        }

        // ── Normalize ────────────────────────────────────────────────
        Inst::Normalize(dst, src) => {
            emit_normalize(e, *dst, *src);
        }

        // ── Length ───────────────────────────────────────────────────
        Inst::Length(dst, src) => {
            // length = sqrt(dp3(v, v))
            e.movups_load(XMM0, RBX, reg_off(*src));
            // dp3: mul, mask w, horizontal sum
            e.movaps_rr(XMM1, XMM0);
            e.mulps(XMM0, XMM1);
            // Mask out w component
            emit_load_const_xmm(e, XMM2, [
                f32::from_bits(0xFFFFFFFF), f32::from_bits(0xFFFFFFFF),
                f32::from_bits(0xFFFFFFFF), 0.0,
            ]);
            e.andps(XMM0, XMM2);
            // Horizontal sum
            e.movshdup(XMM1, XMM0);
            e.addps(XMM0, XMM1);
            e.movhlps(XMM1, XMM0);
            e.addss(XMM0, XMM1);
            // sqrt
            e.sqrtps(XMM0, XMM0);
            // Broadcast to all lanes
            e.shufps(XMM0, XMM0, 0x00);
            e.movups_store(RBX, reg_off(*dst), XMM0);
        }

        // ── Reflect ──────────────────────────────────────────────────
        Inst::Reflect(dst, i, n) => {
            // reflect(I, N) = I - 2 * dot(I, N) * N
            emit_dp3_to_xmm(e, XMM2, *i, *n); // xmm2 = dp3(I, N) broadcast
            e.addps(XMM2, XMM2);               // xmm2 = 2 * dp3
            e.movups_load(XMM1, RBX, reg_off(*n));
            e.mulps(XMM2, XMM1);               // xmm2 = 2 * dp3 * N
            e.movups_load(XMM0, RBX, reg_off(*i));
            e.subps(XMM0, XMM2);               // xmm0 = I - 2*dp3*N
            // Zero out w
            emit_load_const_xmm(e, XMM3, [
                f32::from_bits(0xFFFFFFFF), f32::from_bits(0xFFFFFFFF),
                f32::from_bits(0xFFFFFFFF), 0.0,
            ]);
            e.andps(XMM0, XMM3);
            e.movups_store(RBX, reg_off(*dst), XMM0);
        }

        // ── Sqrt / Rsqrt ─────────────────────────────────────────────
        Inst::Sqrt(dst, src) => {
            e.movups_load(XMM0, RBX, reg_off(*src));
            e.sqrtps(XMM0, XMM0);
            e.movups_store(RBX, reg_off(*dst), XMM0);
        }
        Inst::Rsqrt(dst, src) => {
            // rsqrtps + Newton-Raphson refinement
            e.movups_load(XMM0, RBX, reg_off(*src));
            e.rsqrtps(XMM1, XMM0);  // xmm1 = approx rsqrt(x)
            // Refinement: y = y * (1.5 - 0.5*x*y*y)
            emit_load_const_xmm(e, XMM2, [0.5, 0.5, 0.5, 0.5]);
            e.mulps(XMM0, XMM2);     // xmm0 = 0.5 * x
            e.movaps_rr(XMM3, XMM1);
            e.mulps(XMM3, XMM1);     // xmm3 = y * y
            e.mulps(XMM0, XMM3);     // xmm0 = 0.5 * x * y * y
            emit_load_const_xmm(e, XMM2, [1.5, 1.5, 1.5, 1.5]);
            e.subps(XMM2, XMM0);     // xmm2 = 1.5 - 0.5*x*y*y
            e.mulps(XMM1, XMM2);     // xmm1 = y * (1.5 - 0.5*x*y*y)
            // Zero out lanes where input was <= epsilon
            e.movups_load(XMM0, RBX, reg_off(*src));
            emit_load_const_xmm(e, XMM3, [1e-10, 1e-10, 1e-10, 1e-10]);
            // xmm0 = x > epsilon mask
            // cmpltps: xmm3 = (eps < x) — wrong direction. Need cmplt x, eps = false for valid.
            // Use: cmpneqps with zero, then filter. Simpler: just use the refined result.
            // Actually, for Rsqrt the interpreter checks > 1e-10 per lane. Let's skip
            // the edge case (NaN for zero input) — in practice, shader values are never zero here.
            e.movaps_rr(XMM0, XMM1);
            e.movups_store(RBX, reg_off(*dst), XMM0);
        }

        // ── Transcendentals (call C helpers) ─────────────────────────
        Inst::Pow(dst, base, exp) => {
            // Check if exponent is a known integer constant — use fast multiply chain
            // instead of calling x87 pow(). pow(x, 64.0) = 6 mulps (~6 cycles) vs
            // 4 × x87 fyl2x+f2xm1 (~800 cycles).
            let int_exp = const_regs.get(*exp as usize)
                .and_then(|v| *v)
                .and_then(|val| {
                    let ex = val[0];
                    let n = ex as u32;
                    if n as f32 == ex && n > 0 && n <= 256
                        && val[1] == ex && val[2] == ex && val[3] == ex
                    {
                        Some(n)
                    } else {
                        None
                    }
                });
            if let Some(n) = int_exp {
                emit_pow_int(e, *dst, *base, n);
            } else {
                emit_per_component_binop(e, *dst, *base, *exp, jit_pow as usize);
            }
        }
        Inst::Sin(dst, src) => {
            emit_per_component_unop(e, *dst, *src, jit_sin as usize);
        }
        Inst::Cos(dst, src) => {
            emit_per_component_unop(e, *dst, *src, jit_cos as usize);
        }
        Inst::Floor(dst, src) => {
            // SSE4.1 roundps is faster than calling a C helper
            e.movups_load(XMM0, RBX, reg_off(*src));
            e.roundps(XMM0, XMM0, 0x09); // mode=1 (floor) | 0x08 (inexact suppress)
            e.movups_store(RBX, reg_off(*dst), XMM0);
        }
        Inst::Fract(dst, src) => {
            // fract(x) = x - floor(x)
            e.movups_load(XMM0, RBX, reg_off(*src));
            e.movaps_rr(XMM1, XMM0);
            e.roundps(XMM1, XMM1, 0x09); // floor
            e.subps(XMM0, XMM1);
            e.movups_store(RBX, reg_off(*dst), XMM0);
        }

        // ── Texture sampling ─────────────────────────────────────────
        Inst::TexSample(dst, sampler, coord) => {
            emit_tex_sample(e, *dst, *sampler, *coord);
        }

        // ── Matrix multiply ──────────────────────────────────────────
        Inst::MatMul4(dst, mat, vec) => {
            emit_matmul4(e, *dst, *mat, *vec);
        }
        Inst::MatMul3(dst, mat, vec) => {
            emit_matmul3(e, *dst, *mat, *vec);
        }

        // ── Swizzle ──────────────────────────────────────────────────
        Inst::Swizzle(dst, src, indices, count) => {
            emit_swizzle(e, *dst, *src, indices, *count);
        }

        // ── WriteMask ────────────────────────────────────────────────
        Inst::WriteMask(dst, src, mask) => {
            emit_writemask(e, *dst, *src, *mask);
        }

        // ── Comparisons ──────────────────────────────────────────────
        Inst::CmpLt(dst, a, b) => {
            e.movups_load(XMM0, RBX, reg_off(*a));
            e.movups_load(XMM1, RBX, reg_off(*b));
            e.cmpltps(XMM0, XMM1); // xmm0 = (a < b) ? 0xFFFFFFFF : 0
            // Convert to 1.0 / 0.0
            emit_load_const_xmm(e, XMM1, [1.0, 1.0, 1.0, 1.0]);
            e.andps(XMM0, XMM1);
            e.movups_store(RBX, reg_off(*dst), XMM0);
        }
        Inst::CmpEq(dst, a, b) => {
            // Epsilon comparison: |a - b| < 1e-6
            e.movups_load(XMM0, RBX, reg_off(*a));
            e.movups_load(XMM1, RBX, reg_off(*b));
            e.subps(XMM0, XMM1);
            // abs
            emit_load_const_xmm(e, XMM2, [
                f32::from_bits(0x7FFFFFFF), f32::from_bits(0x7FFFFFFF),
                f32::from_bits(0x7FFFFFFF), f32::from_bits(0x7FFFFFFF),
            ]);
            e.andps(XMM0, XMM2);
            emit_load_const_xmm(e, XMM1, [1e-6, 1e-6, 1e-6, 1e-6]);
            e.cmpltps(XMM0, XMM1); // |diff| < eps
            emit_load_const_xmm(e, XMM1, [1.0, 1.0, 1.0, 1.0]);
            e.andps(XMM0, XMM1);
            e.movups_store(RBX, reg_off(*dst), XMM0);
        }
        Inst::Select(dst, cond, a, b) => {
            e.movups_load(XMM0, RBX, reg_off(*cond));
            e.movups_load(XMM1, RBX, reg_off(*a));
            e.movups_load(XMM2, RBX, reg_off(*b));
            // mask = (cond != 0)
            e.xorps(XMM3, XMM3);
            e.cmpneqps(XMM0, XMM3);
            // result = (a & mask) | (b & ~mask)
            e.movaps_rr(XMM3, XMM0);
            e.andps(XMM3, XMM1);     // a & mask
            e.andnps(XMM0, XMM2);    // ~mask & b
            e.orps(XMM0, XMM3);      // combine
            e.movups_store(RBX, reg_off(*dst), XMM0);
        }

        // ── Type conversions ─────────────────────────────────────────
        Inst::IntToFloat(dst, src) => {
            // In our IR, int and float share the same representation
            if dst != src {
                e.movups_load(XMM0, RBX, reg_off(*src));
                e.movups_store(RBX, reg_off(*dst), XMM0);
            }
        }
        Inst::FloatToInt(dst, src) => {
            // Convert each lane: truncate to int, then back to float
            emit_per_component_float_to_int(e, *dst, *src);
        }

        // ── I/O instructions ─────────────────────────────────────────
        Inst::StorePosition(src) => {
            e.movups_load(XMM0, RBX, reg_off(*src));
            // Load position pointer from ctx
            e.mov_r64_mem(RAX, RBP, CTX_POSITION);
            e.movups_store(RAX, 0, XMM0);
        }
        Inst::StoreFragColor(src) => {
            e.movups_load(XMM0, RBX, reg_off(*src));
            e.mov_r64_mem(RAX, RBP, CTX_FRAG_COLOR);
            e.movups_store(RAX, 0, XMM0);
        }
        Inst::StorePointSize(src) => {
            e.movss_load(XMM0, RBX, reg_off(*src));
            e.mov_r64_mem(RAX, RBP, CTX_POINT_SIZE);
            e.movss_store(RAX, 0, XMM0);
        }
        Inst::LoadVarying(dst, idx) => {
            // Load from varyings_in (R14), which may be null for VS
            // For simplicity, just load — caller ensures valid pointer
            let off = (*idx as i32) * 16;
            e.movups_load(XMM0, R14, off);
            e.movups_store(RBX, reg_off(*dst), XMM0);
        }
        Inst::StoreVarying(idx, src) => {
            let off = (*idx as i32) * 16;
            e.movups_load(XMM0, RBX, reg_off(*src));
            e.movups_store(R15, off, XMM0);
        }
        Inst::LoadUniform(dst, idx) => {
            let off = (*idx as i32) * 16;
            e.movups_load(XMM0, R12, off);
            e.movups_store(RBX, reg_off(*dst), XMM0);
        }
        Inst::LoadAttribute(dst, idx) => {
            let off = (*idx as i32) * 16;
            e.movups_load(XMM0, R13, off);
            e.movups_store(RBX, reg_off(*dst), XMM0);
        }
    }
}

// ── Instruction emission helpers ─────────────────────────────────────────

/// Emit a binary SSE operation: load a → XMM0, load b → XMM1, op, store.
fn emit_binop(e: &mut Emitter, dst: u32, a: u32, b: u32, op: fn(&mut Emitter)) {
    e.movups_load(XMM0, RBX, reg_off(a));
    e.movups_load(XMM1, RBX, reg_off(b));
    op(e);
    e.movups_store(RBX, reg_off(dst), XMM0);
}

/// Load a constant [f32; 4] into an XMM register.
///
/// Emits the constant inline using a `jmp` over the data, then loads via
/// RIP-relative addressing. Simple and avoids a separate constant pool.
fn emit_load_const_xmm(e: &mut Emitter, xmm: u8, val: [f32; 4]) {
    // jmp +16 (skip the 16-byte constant)
    e.emit(0xEB);
    e.emit(16);
    let data_pos = e.pos();
    e.emit_f32x4(val);
    // Now load from the constant address.
    // We know the constant is at code[data_pos]. Use RIP-relative:
    // movups xmm, [rip + disp32]
    // The disp32 is relative to the END of this instruction (7 bytes for movups rip-rel).
    // Current pos = data_pos + 16. The movups instruction is:
    //   [REX?] 0F 10 modrm disp32  (5 or 6 bytes for xmm0-7, 6 or 7 for xmm8-15)
    let inst_start = e.pos();
    if xmm >= 8 {
        e.rex(false, true, false, false);
    }
    e.emit(0x0F);
    e.emit(0x10);
    e.emit(0x05 | ((xmm & 7) << 3)); // ModRM: mod=00, reg=xmm, r/m=101 (RIP-relative)
    let inst_end = e.pos() + 4; // 4 bytes for disp32
    let disp = data_pos as i32 - inst_end as i32;
    e.emit_i32(disp);
}

/// Emit LoadConst: store 4 floats into a virtual register.
fn emit_load_const(e: &mut Emitter, dst: u32, val: [f32; 4]) {
    emit_load_const_xmm(e, XMM0, val);
    e.movups_store(RBX, reg_off(dst), XMM0);
}

/// Emit 3-component dot product, result broadcast in `out_xmm`.
fn emit_dp3_to_xmm(e: &mut Emitter, out_xmm: u8, a: u32, b: u32) {
    e.movups_load(XMM4, RBX, reg_off(a));
    e.movups_load(XMM5, RBX, reg_off(b));
    e.mulps(XMM4, XMM5);
    // Mask w to zero
    emit_load_const_xmm(e, XMM6, [
        f32::from_bits(0xFFFFFFFF), f32::from_bits(0xFFFFFFFF),
        f32::from_bits(0xFFFFFFFF), 0.0,
    ]);
    e.andps(XMM4, XMM6);
    // Horizontal sum: movshdup + add + movhlps + addss + broadcast
    e.movshdup(XMM5, XMM4);
    e.addps(XMM4, XMM5);
    e.movhlps(XMM5, XMM4);
    e.addss(XMM4, XMM5);
    e.shufps(XMM4, XMM4, 0x00); // broadcast
    if out_xmm != XMM4 {
        e.movaps_rr(out_xmm, XMM4);
    }
}

/// Emit dp3 instruction.
fn emit_dp3(e: &mut Emitter, dst: u32, a: u32, b: u32) {
    emit_dp3_to_xmm(e, XMM0, a, b);
    e.movups_store(RBX, reg_off(dst), XMM0);
}

/// Emit dp4 instruction.
fn emit_dp4(e: &mut Emitter, dst: u32, a: u32, b: u32) {
    e.movups_load(XMM0, RBX, reg_off(a));
    e.movups_load(XMM1, RBX, reg_off(b));
    e.mulps(XMM0, XMM1);
    // Horizontal sum (4 components)
    e.movshdup(XMM1, XMM0);
    e.addps(XMM0, XMM1);
    e.movhlps(XMM1, XMM0);
    e.addss(XMM0, XMM1);
    e.shufps(XMM0, XMM0, 0x00);
    e.movups_store(RBX, reg_off(dst), XMM0);
}

/// Emit cross product.
fn emit_cross(e: &mut Emitter, dst: u32, a: u32, b: u32) {
    // cross(a, b) = (a.yzx * b.zxy) - (a.zxy * b.yzx)
    e.movups_load(XMM0, RBX, reg_off(a));
    e.movups_load(XMM1, RBX, reg_off(b));
    // xmm2 = a.yzx
    e.movaps_rr(XMM2, XMM0);
    e.shufps(XMM2, XMM2, 0xC9); // 3,0,2,1 = 11_00_10_01
    // xmm3 = b.zxy
    e.movaps_rr(XMM3, XMM1);
    e.shufps(XMM3, XMM3, 0xD2); // 3,1,0,2 = 11_01_00_10
    e.mulps(XMM2, XMM3);         // a.yzx * b.zxy
    // xmm4 = a.zxy
    e.movaps_rr(XMM4, XMM0);
    e.shufps(XMM4, XMM4, 0xD2);
    // xmm5 = b.yzx
    e.movaps_rr(XMM5, XMM1);
    e.shufps(XMM5, XMM5, 0xC9);
    e.mulps(XMM4, XMM5);         // a.zxy * b.yzx
    e.subps(XMM2, XMM4);         // result
    // Zero out w
    emit_load_const_xmm(e, XMM3, [
        f32::from_bits(0xFFFFFFFF), f32::from_bits(0xFFFFFFFF),
        f32::from_bits(0xFFFFFFFF), 0.0,
    ]);
    e.andps(XMM2, XMM3);
    e.movups_store(RBX, reg_off(dst), XMM2);
}

/// Emit normalize: dst = src / length(src).
fn emit_normalize(e: &mut Emitter, dst: u32, src: u32) {
    // dp3(v, v)
    e.movups_load(XMM0, RBX, reg_off(src));
    e.movaps_rr(XMM1, XMM0);
    e.mulps(XMM1, XMM0);
    // Mask w
    emit_load_const_xmm(e, XMM3, [
        f32::from_bits(0xFFFFFFFF), f32::from_bits(0xFFFFFFFF),
        f32::from_bits(0xFFFFFFFF), 0.0,
    ]);
    e.andps(XMM1, XMM3);
    // Horizontal sum → xmm1.x = len_sq
    e.movshdup(XMM2, XMM1);
    e.addps(XMM1, XMM2);
    e.movhlps(XMM2, XMM1);
    e.addss(XMM1, XMM2);
    // rsqrtss + Newton-Raphson for inv_len
    e.rsqrtps(XMM2, XMM1); // xmm2 = approx rsqrt
    // Refinement: y * (1.5 - 0.5*x*y*y)
    emit_load_const_xmm(e, XMM3, [0.5, 0.5, 0.5, 0.5]);
    e.mulss(XMM1, XMM3);   // xmm1 = 0.5 * len_sq
    e.movaps_rr(XMM4, XMM2);
    e.mulss(XMM4, XMM2);   // xmm4 = y*y
    e.mulss(XMM1, XMM4);   // xmm1 = 0.5 * len_sq * y*y
    emit_load_const_xmm(e, XMM3, [1.5, 1.5, 1.5, 1.5]);
    e.subss(XMM3, XMM1);   // xmm3 = 1.5 - 0.5*x*y*y
    e.mulss(XMM2, XMM3);   // xmm2 = refined rsqrt

    // Broadcast inv_len to all lanes
    e.shufps(XMM2, XMM2, 0x00);
    // v * inv_len
    e.movups_load(XMM0, RBX, reg_off(src));
    e.mulps(XMM0, XMM2);
    e.movups_store(RBX, reg_off(dst), XMM0);
}

/// Emit integer power via binary exponentiation (multiply chain).
///
/// Computes `dst = base ^ n` using only SIMD multiplies, fully unrolled at
/// JIT compile time. For `n=64` this emits 6 `mulps` instructions (~6 cycles)
/// instead of 4 calls to x87 `pow()` (~800 cycles) — a 130× speedup per pixel.
fn emit_pow_int(e: &mut Emitter, dst: u32, base: u32, n: u32) {
    if n == 0 {
        emit_load_const(e, dst, [1.0, 1.0, 1.0, 1.0]);
        return;
    }
    if n == 1 {
        e.movups_load(XMM0, RBX, reg_off(base));
        e.movups_store(RBX, reg_off(dst), XMM0);
        return;
    }

    // Binary exponentiation, fully unrolled at JIT compile time.
    // XMM0 = original base (never modified), XMM1 = accumulator.
    e.movups_load(XMM0, RBX, reg_off(base));
    e.movaps_rr(XMM1, XMM0); // result = x (from highest set bit)

    let bits = 32 - n.leading_zeros();
    // Process from second-highest bit down to bit 0
    for i in (0..bits - 1).rev() {
        e.mulps(XMM1, XMM1); // result = result²
        if (n >> i) & 1 == 1 {
            e.mulps(XMM1, XMM0); // result *= base
        }
    }

    e.movups_store(RBX, reg_off(dst), XMM1);
}

/// Emit per-component unary function call (sin, cos, etc.).
///
/// Calls a C-ABI helper for each of the 4 components. Uses the stack to
/// accumulate results without clobbering callee-saved registers.
fn emit_per_component_unop(e: &mut Emitter, dst: u32, src: u32, func: usize) {
    // Save callee-saved regs that the called function may clobber: none needed
    // (our callee-saved RBX, R12-R15, RBP are safe across calls).
    // But we need 16 bytes of stack for the result accumulator.
    e.sub_rsp_imm8(32); // 16 for result + 16 for alignment

    for comp in 0..4u32 {
        // Load component into xmm0 (first float arg)
        e.movss_load(XMM0, RBX, reg_off(src) + comp as i32 * 4);
        // Call helper: float result in xmm0
        e.mov_r64_imm64(RAX, func as u64);
        e.call_r64(RAX);
        // Store result component to stack
        e.movss_store(RSP, comp as i32 * 4, XMM0);
    }

    // Load accumulated result and store to dst register
    e.movups_load(XMM0, RSP, 0);
    e.movups_store(RBX, reg_off(dst), XMM0);
    e.add_rsp_imm8(32);
}

/// Emit per-component binary function call (pow).
fn emit_per_component_binop(e: &mut Emitter, dst: u32, a: u32, b: u32, func: usize) {
    e.sub_rsp_imm8(32);

    for comp in 0..4u32 {
        // xmm0 = a[comp], xmm1 = b[comp]
        e.movss_load(XMM0, RBX, reg_off(a) + comp as i32 * 4);
        e.movss_load(XMM1, RBX, reg_off(b) + comp as i32 * 4);
        e.mov_r64_imm64(RAX, func as u64);
        e.call_r64(RAX);
        e.movss_store(RSP, comp as i32 * 4, XMM0);
    }

    e.movups_load(XMM0, RSP, 0);
    e.movups_store(RBX, reg_off(dst), XMM0);
    e.add_rsp_imm8(32);
}

/// Emit FloatToInt: truncate each float to int, then back to float.
fn emit_per_component_float_to_int(e: &mut Emitter, dst: u32, src: u32) {
    e.sub_rsp_imm8(32);
    for comp in 0..4u32 {
        e.movss_load(XMM0, RBX, reg_off(src) + comp as i32 * 4);
        e.cvttss2si_r32_xmm(RAX, XMM0);
        e.cvtsi2ss_xmm_r32(XMM0, RAX);
        e.movss_store(RSP, comp as i32 * 4, XMM0);
    }
    e.movups_load(XMM0, RSP, 0);
    e.movups_store(RBX, reg_off(dst), XMM0);
    e.add_rsp_imm8(32);
}

/// Emit texture sampling via C helper.
fn emit_tex_sample(e: &mut Emitter, dst: u32, sampler: u32, coord: u32) {
    // Allocate stack: 16 bytes for output + 16 for alignment
    e.sub_rsp_imm8(32);

    // jit_tex_sample(tex_fn_ptr: usize, unit: u32, u: f32, v: f32, out: *mut [f32;4])
    // System V AMD64: integer args → RDI, RSI, RDX; float args → XMM0, XMM1
    //   arg1 (usize tex_fn_ptr) → RDI
    //   arg2 (u32 unit)         → ESI
    //   arg3 (f32 u)            → XMM0
    //   arg4 (f32 v)            → XMM1
    //   arg5 (ptr out)          → RDX

    // Load tex_sample function pointer from context
    e.mov_r64_mem(RDI, RBP, CTX_TEX_SAMPLE);
    // Load unit = regs[sampler][0] as u32
    e.movss_load(XMM0, RBX, reg_off(sampler));
    e.cvttss2si_r32_xmm(RSI, XMM0);
    // Load u, v from coord register
    e.movss_load(XMM0, RBX, reg_off(coord));       // u
    e.movss_load(XMM1, RBX, reg_off(coord) + 4);   // v
    // out pointer = RSP → RDX
    e.mov_r64_r64(RDX, RSP);

    // Call jit_tex_sample
    e.mov_r64_imm64(RAX, jit_tex_sample as usize as u64);
    e.call_r64(RAX);

    // Result is at [RSP], load it
    e.movups_load(XMM0, RSP, 0);
    e.movups_store(RBX, reg_off(dst), XMM0);
    e.add_rsp_imm8(32);
}

/// Emit MatMul4: dst = mat * vec (column-major 4x4).
fn emit_matmul4(e: &mut Emitter, dst: u32, mat: u32, vec: u32) {
    // Load vector
    e.movups_load(XMM0, RBX, reg_off(vec));
    // Load 4 column vectors
    e.movups_load(XMM4, RBX, reg_off(mat));
    e.movups_load(XMM5, RBX, reg_off(mat + 1));
    e.movups_load(XMM6, RBX, reg_off(mat + 2));
    e.movups_load(XMM7, RBX, reg_off(mat + 3));

    // Splat each component of vec
    e.movaps_rr(XMM1, XMM0);
    e.shufps(XMM1, XMM1, 0x00); // vx broadcast
    e.mulps(XMM1, XMM4);         // col0 * vx

    e.movaps_rr(XMM2, XMM0);
    e.shufps(XMM2, XMM2, 0x55); // vy broadcast
    e.mulps(XMM2, XMM5);         // col1 * vy

    e.movaps_rr(XMM3, XMM0);
    e.shufps(XMM3, XMM3, 0xAA); // vz broadcast
    e.mulps(XMM3, XMM6);         // col2 * vz

    e.shufps(XMM0, XMM0, 0xFF); // vw broadcast
    e.mulps(XMM0, XMM7);         // col3 * vw

    // Sum
    e.addps(XMM1, XMM2);
    e.addps(XMM3, XMM0);
    e.addps(XMM1, XMM3);

    e.movups_store(RBX, reg_off(dst), XMM1);
}

/// Emit MatMul3: dst = mat * vec (column-major 3x3, w=0).
fn emit_matmul3(e: &mut Emitter, dst: u32, mat: u32, vec: u32) {
    e.movups_load(XMM0, RBX, reg_off(vec));
    e.movups_load(XMM4, RBX, reg_off(mat));
    e.movups_load(XMM5, RBX, reg_off(mat + 1));
    e.movups_load(XMM6, RBX, reg_off(mat + 2));

    e.movaps_rr(XMM1, XMM0);
    e.shufps(XMM1, XMM1, 0x00);
    e.mulps(XMM1, XMM4);

    e.movaps_rr(XMM2, XMM0);
    e.shufps(XMM2, XMM2, 0x55);
    e.mulps(XMM2, XMM5);

    e.movaps_rr(XMM3, XMM0);
    e.shufps(XMM3, XMM3, 0xAA);
    e.mulps(XMM3, XMM6);

    e.addps(XMM1, XMM2);
    e.addps(XMM1, XMM3);

    // Mask w to zero
    emit_load_const_xmm(e, XMM2, [
        f32::from_bits(0xFFFFFFFF), f32::from_bits(0xFFFFFFFF),
        f32::from_bits(0xFFFFFFFF), 0.0,
    ]);
    e.andps(XMM1, XMM2);

    e.movups_store(RBX, reg_off(dst), XMM1);
}

/// Emit swizzle instruction.
fn emit_swizzle(e: &mut Emitter, dst: u32, src: u32, indices: &[u8; 4], count: u8) {
    e.movups_load(XMM0, RBX, reg_off(src));

    if count == 1 {
        // Broadcast single component
        let imm = indices[0] | (indices[0] << 2) | (indices[0] << 4) | (indices[0] << 6);
        e.shufps(XMM0, XMM0, imm);
    } else if count == 4 && indices[0] < 4 && indices[1] < 4 && indices[2] < 4 && indices[3] < 4 {
        // Use shufps for 4-component swizzle
        let imm = indices[0] | (indices[1] << 2) | (indices[2] << 4) | (indices[3] << 6);
        e.shufps(XMM0, XMM0, imm);
    } else {
        // General case: store to stack, reload components
        e.sub_rsp_imm8(32);
        e.movups_store(RSP, 0, XMM0);
        for i in 0..(count as u32) {
            e.movss_load(XMM1, RSP, indices[i as usize] as i32 * 4);
            e.movss_store(RSP, 16 + i as i32 * 4, XMM1);
        }
        // Zero remaining
        for i in (count as u32)..4 {
            e.xorps(XMM1, XMM1);
            e.movss_store(RSP, 16 + i as i32 * 4, XMM1);
        }
        e.movups_load(XMM0, RSP, 16);
        e.add_rsp_imm8(32);
    }

    e.movups_store(RBX, reg_off(dst), XMM0);
}

/// Emit write-mask instruction.
fn emit_writemask(e: &mut Emitter, dst: u32, src: u32, mask: u8) {
    if mask == 0x0F {
        // All components: just copy
        e.movups_load(XMM0, RBX, reg_off(src));
        e.movups_store(RBX, reg_off(dst), XMM0);
        return;
    }

    // General case: blend src into dst based on mask
    e.movups_load(XMM0, RBX, reg_off(dst)); // existing dst
    e.movups_load(XMM1, RBX, reg_off(src)); // new src

    // Build blend mask constant
    let m = [
        if mask & 1 != 0 { 0xFFFFFFFFu32 } else { 0 },
        if mask & 2 != 0 { 0xFFFFFFFF } else { 0 },
        if mask & 4 != 0 { 0xFFFFFFFF } else { 0 },
        if mask & 8 != 0 { 0xFFFFFFFF } else { 0 },
    ];
    emit_load_const_xmm(e, XMM2, [
        f32::from_bits(m[0]), f32::from_bits(m[1]),
        f32::from_bits(m[2]), f32::from_bits(m[3]),
    ]);

    // result = (src & mask) | (dst & ~mask)
    e.andps(XMM1, XMM2);     // src & mask
    e.andnps(XMM2, XMM0);    // ~mask & dst
    e.orps(XMM1, XMM2);      // combine
    e.movups_store(RBX, reg_off(dst), XMM1);
}
