//! x86-64 machine code emitter for JIT compilation.
//!
//! A minimal assembler that produces raw x86-64 byte sequences. Designed for
//! same-ISA dynamic binary translation: guest x86 → host x86-64.
//!
//! Supports the instruction subset needed for JIT Phase 3:
//! - Data movement (MOV, MOVZX, MOVSX, LEA, PUSH, POP, XCHG)
//! - Arithmetic (ADD, SUB, CMP, INC, DEC, NEG, IMUL)
//! - Logic (AND, OR, XOR, TEST, NOT, SHL, SHR, SAR)
//! - Control flow (JMP, Jcc, CALL, RET)
//! - Flags (PUSHFQ, POPFQ, LAHF, SAHF)
//!
//! All instructions encode the correct REX prefix for 64-bit operands and
//! extended registers (R8-R15).

use alloc::vec::Vec;

// ── Host register encoding ─────────────────────────────────────────────

/// x86-64 register index (0-15).
///
/// Matches hardware encoding: 0=RAX, 1=RCX, 2=RDX, 3=RBX, 4=RSP,
/// 5=RBP, 6=RSI, 7=RDI, 8=R8 .. 15=R15.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Reg {
    Rax = 0, Rcx = 1, Rdx = 2, Rbx = 3,
    Rsp = 4, Rbp = 5, Rsi = 6, Rdi = 7,
    R8 = 8,  R9 = 9,  R10 = 10, R11 = 11,
    R12 = 12, R13 = 13, R14 = 14, R15 = 15,
}

impl Reg {
    /// Low 3 bits of the register encoding.
    #[inline]
    fn lo3(self) -> u8 { self as u8 & 7 }

    /// True if register index >= 8 (needs REX extension bit).
    #[inline]
    fn is_ext(self) -> bool { self as u8 >= 8 }
}

// ── Condition codes ────────────────────────────────────────────────────

/// x86 condition codes for Jcc and SETcc instructions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Cc {
    O = 0x0,   No = 0x1,  B = 0x2,   Ae = 0x3,
    E = 0x4,   Ne = 0x5,  Be = 0x6,  A = 0x7,
    S = 0x8,   Ns = 0x9,  P = 0xA,   Np = 0xB,
    L = 0xC,   Ge = 0xD,  Le = 0xE,  G = 0xF,
}

// ── Operand size ───────────────────────────────────────────────────────

/// Operand size for JIT instructions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpSize {
    /// 8-bit (byte).
    S8,
    /// 16-bit (word).
    S16,
    /// 32-bit (dword) — default, zero-extends to 64-bit in x86-64.
    S32,
    /// 64-bit (qword) — requires REX.W.
    S64,
}

// ── Label for forward references ───────────────────────────────────────

/// Opaque label ID for forward jump references.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Label(u32);

/// A pending fixup: location in the code buffer where a rel32 displacement
/// must be patched once the target label is defined.
struct Fixup {
    /// Label this fixup refers to.
    label: Label,
    /// Byte offset in `code` where the 4-byte rel32 starts.
    patch_offset: usize,
    /// Byte offset in `code` immediately after the rel32 field (= IP at
    /// the time the jump executes, since rel32 is relative to the next IP).
    next_ip_offset: usize,
}

// ── Emitter ────────────────────────────────────────────────────────────

/// x86-64 machine code emitter.
///
/// Accumulates instruction bytes into an internal buffer. After all
/// instructions are emitted, call [`finalize`] to resolve labels and
/// return the raw machine code.
pub struct Emitter {
    /// Accumulated machine code bytes.
    code: Vec<u8>,
    /// Defined label positions: label ID → byte offset in `code`.
    labels: Vec<Option<usize>>,
    /// Pending fixups for forward label references.
    fixups: Vec<Fixup>,
}

impl Emitter {
    /// Create a new emitter with an empty code buffer.
    pub fn new() -> Self {
        Emitter {
            code: Vec::with_capacity(4096),
            labels: Vec::new(),
            fixups: Vec::new(),
        }
    }

    /// Current code buffer length (= offset of next emitted byte).
    #[inline]
    pub fn offset(&self) -> usize {
        self.code.len()
    }

    /// Allocate a new label (initially undefined).
    pub fn new_label(&mut self) -> Label {
        let id = self.labels.len() as u32;
        self.labels.push(None);
        Label(id)
    }

    /// Define a label at the current code offset.
    pub fn bind_label(&mut self, label: Label) {
        self.labels[label.0 as usize] = Some(self.code.len());
    }

    /// Return the bound offset of a label. Panics if the label is not yet defined.
    pub fn label_offset(&self, label: Label) -> usize {
        self.labels[label.0 as usize].expect("JIT emitter: label not yet bound")
    }

    /// Finalize: resolve all forward label references and return raw code.
    ///
    /// Panics if any label referenced by a fixup has not been defined.
    pub fn finalize(mut self) -> Vec<u8> {
        for fixup in &self.fixups {
            let target = self.labels[fixup.label.0 as usize]
                .expect("JIT emitter: unresolved label");
            let rel = (target as i64) - (fixup.next_ip_offset as i64);
            let rel32 = rel as i32;
            let bytes = rel32.to_le_bytes();
            self.code[fixup.patch_offset..fixup.patch_offset + 4]
                .copy_from_slice(&bytes);
        }
        self.code
    }

    // ── Raw byte emission ──────────────────────────────────────────────

    #[inline]
    fn emit(&mut self, byte: u8) {
        self.code.push(byte);
    }

    /// Emit raw bytes directly. Use sparingly for instructions not yet
    /// wrapped as dedicated methods.
    #[inline]
    pub fn emit_raw(&mut self, bytes: &[u8]) {
        self.code.extend_from_slice(bytes);
    }

    #[inline]
    fn emit_u32_le(&mut self, val: u32) {
        self.code.extend_from_slice(&val.to_le_bytes());
    }

    #[inline]
    fn emit_u64_le(&mut self, val: u64) {
        self.code.extend_from_slice(&val.to_le_bytes());
    }

    #[inline]
    fn emit_i32_le(&mut self, val: i32) {
        self.code.extend_from_slice(&val.to_le_bytes());
    }

    // ── REX prefix helpers ─────────────────────────────────────────────

    /// Emit a REX prefix if needed.
    ///
    /// `w`: set REX.W (64-bit operand)
    /// `reg_ext`: set REX.R (reg field extension)
    /// `rm_ext`: set REX.B (rm/base field extension)
    /// `index_ext`: set REX.X (SIB index extension)
    fn emit_rex(&mut self, w: bool, reg_ext: bool, index_ext: bool, rm_ext: bool) {
        let byte = 0x40
            | if w { 8 } else { 0 }
            | if reg_ext { 4 } else { 0 }
            | if index_ext { 2 } else { 0 }
            | if rm_ext { 1 } else { 0 };
        self.emit(byte);
    }

    /// Emit a REX prefix for reg-reg operations if required by the operands
    /// or operand size.
    fn maybe_rex_rr(&mut self, size: OpSize, reg: Reg, rm: Reg) {
        let w = size == OpSize::S64;
        let r = reg.is_ext();
        let b = rm.is_ext();
        if w || r || b {
            self.emit_rex(w, r, false, b);
        }
    }

    /// Emit a REX prefix for [base+disp] memory operations if required.
    fn maybe_rex_rm(&mut self, size: OpSize, reg: Reg, base: Reg) {
        // Same as reg-reg: R extends reg, B extends base (rm field).
        self.maybe_rex_rr(size, reg, base);
    }

    /// Emit a REX prefix for single-operand instructions.
    fn maybe_rex_single(&mut self, size: OpSize, rm: Reg) {
        let w = size == OpSize::S64;
        let b = rm.is_ext();
        if w || b {
            self.emit_rex(w, false, false, b);
        }
    }

    // ── ModR/M + SIB encoding ──────────────────────────────────────────

    /// Emit ModR/M byte for register-register (mod=11).
    #[inline]
    fn modrm_rr(&mut self, reg: Reg, rm: Reg) {
        self.emit(0xC0 | (reg.lo3() << 3) | rm.lo3());
    }

    /// Emit ModR/M + optional SIB + displacement for [base + disp32].
    ///
    /// Handles special cases: RSP/R12 requires SIB, RBP/R13 with disp=0
    /// still needs disp8=0.
    fn modrm_mem(&mut self, reg: Reg, base: Reg, disp: i32) {
        let rm = base.lo3();
        let reg_bits = reg.lo3();

        // Determine mod field from displacement magnitude.
        let (mod_bits, disp_size) = if disp == 0 && rm != 5 {
            // mod=00: no displacement (except RBP/R13 which MUST use disp8)
            (0u8, 0u8)
        } else if disp >= -128 && disp <= 127 {
            // mod=01: 8-bit displacement
            (1u8, 1u8)
        } else {
            // mod=10: 32-bit displacement
            (2u8, 4u8)
        };

        if rm == 4 {
            // RSP/R12: must use SIB byte. SIB = [ss=00][index=RSP(none)][base=RSP]
            self.emit((mod_bits << 6) | (reg_bits << 3) | 4);
            self.emit(0x24); // SIB: scale=1, index=RSP(none), base=RSP
        } else {
            self.emit((mod_bits << 6) | (reg_bits << 3) | rm);
        }

        match disp_size {
            1 => self.emit(disp as u8),
            4 => self.emit_i32_le(disp),
            _ => {}
        }
    }

    /// Emit ModR/M for a /digit instruction (e.g. `add [base+disp], imm` uses `/0`).
    fn modrm_mem_digit(&mut self, digit: u8, base: Reg, disp: i32) {
        // The digit occupies the reg field position.
        self.modrm_mem(
            // Safety: digit 0-7 maps to Rax-Rdi encodings.
            unsafe { core::mem::transmute::<u8, Reg>(digit) },
            base,
            disp,
        );
    }

    // ── Operand size prefix ────────────────────────────────────────────

    fn maybe_size_prefix(&mut self, size: OpSize) {
        if size == OpSize::S16 {
            self.emit(0x66); // Operand-size override
        }
    }

    // ════════════════════════════════════════════════════════════════════
    // Data Movement
    // ════════════════════════════════════════════════════════════════════

    /// `MOV dst, src` (register to register).
    pub fn mov_rr(&mut self, size: OpSize, dst: Reg, src: Reg) {
        self.maybe_size_prefix(size);
        match size {
            OpSize::S8 => {
                self.maybe_rex_rr(OpSize::S8, src, dst);
                self.emit(0x88);
            }
            _ => {
                self.maybe_rex_rr(size, src, dst);
                self.emit(0x89);
            }
        }
        self.modrm_rr(src, dst);
    }

    /// `MOV dst, imm64` (64-bit immediate to register).
    pub fn mov_ri64(&mut self, dst: Reg, imm: u64) {
        self.emit_rex(true, false, false, dst.is_ext());
        self.emit(0xB8 + dst.lo3());
        self.emit_u64_le(imm);
    }

    /// `MOV dst, imm32` (32-bit immediate to register, zero-extends).
    pub fn mov_ri32(&mut self, dst: Reg, imm: u32) {
        if dst.is_ext() {
            self.emit_rex(false, false, false, true);
        }
        self.emit(0xB8 + dst.lo3());
        self.emit_u32_le(imm);
    }

    /// `MOV dst, [base + disp]` (memory to register).
    pub fn mov_rm(&mut self, size: OpSize, dst: Reg, base: Reg, disp: i32) {
        self.maybe_size_prefix(size);
        match size {
            OpSize::S8 => {
                self.maybe_rex_rm(OpSize::S8, dst, base);
                self.emit(0x8A);
            }
            _ => {
                self.maybe_rex_rm(size, dst, base);
                self.emit(0x8B);
            }
        }
        self.modrm_mem(dst, base, disp);
    }

    /// `MOV [base + disp], src` (register to memory).
    pub fn mov_mr(&mut self, size: OpSize, base: Reg, disp: i32, src: Reg) {
        self.maybe_size_prefix(size);
        match size {
            OpSize::S8 => {
                self.maybe_rex_rm(OpSize::S8, src, base);
                self.emit(0x88);
            }
            _ => {
                self.maybe_rex_rm(size, src, base);
                self.emit(0x89);
            }
        }
        self.modrm_mem(src, base, disp);
    }

    /// `MOV [base + disp], imm32` (sign-extended immediate to memory).
    pub fn mov_mi32(&mut self, size: OpSize, base: Reg, disp: i32, imm: i32) {
        self.maybe_size_prefix(size);
        self.maybe_rex_rm(size, Reg::Rax, base); // /0 digit uses Rax encoding
        self.emit(0xC7);
        self.modrm_mem_digit(0, base, disp);
        if size == OpSize::S16 {
            self.code.extend_from_slice(&(imm as i16).to_le_bytes());
        } else {
            self.emit_i32_le(imm);
        }
    }

    /// `MOVZX dst, BYTE [base + disp]` (zero-extend byte to 32/64-bit).
    pub fn movzx_rm8(&mut self, size: OpSize, dst: Reg, base: Reg, disp: i32) {
        self.maybe_rex_rm(size, dst, base);
        self.emit(0x0F);
        self.emit(0xB6);
        self.modrm_mem(dst, base, disp);
    }

    /// `MOVZX dst, WORD [base + disp]` (zero-extend word to 32/64-bit).
    pub fn movzx_rm16(&mut self, size: OpSize, dst: Reg, base: Reg, disp: i32) {
        self.maybe_rex_rm(size, dst, base);
        self.emit(0x0F);
        self.emit(0xB7);
        self.modrm_mem(dst, base, disp);
    }

    /// `MOVSX dst, BYTE [base + disp]` (sign-extend byte).
    pub fn movsx_rm8(&mut self, size: OpSize, dst: Reg, base: Reg, disp: i32) {
        self.maybe_rex_rm(size, dst, base);
        self.emit(0x0F);
        self.emit(0xBE);
        self.modrm_mem(dst, base, disp);
    }

    /// `MOVSX dst, WORD [base + disp]` (sign-extend word).
    pub fn movsx_rm16(&mut self, size: OpSize, dst: Reg, base: Reg, disp: i32) {
        self.maybe_rex_rm(size, dst, base);
        self.emit(0x0F);
        self.emit(0xBF);
        self.modrm_mem(dst, base, disp);
    }

    /// `MOVSXD dst, DWORD [base + disp]` (sign-extend dword to qword, REX.W).
    pub fn movsxd_rm(&mut self, dst: Reg, base: Reg, disp: i32) {
        self.maybe_rex_rm(OpSize::S64, dst, base);
        self.emit(0x63);
        self.modrm_mem(dst, base, disp);
    }

    /// `MOVZX dst, src_byte` (register-register, zero-extend byte).
    pub fn movzx_rr8(&mut self, size: OpSize, dst: Reg, src: Reg) {
        self.maybe_rex_rr(size, dst, src);
        self.emit(0x0F);
        self.emit(0xB6);
        self.modrm_rr(dst, src);
    }

    /// `MOVSX dst, src_byte` (register-register, sign-extend byte).
    pub fn movsx_rr8(&mut self, size: OpSize, dst: Reg, src: Reg) {
        self.maybe_rex_rr(size, dst, src);
        self.emit(0x0F);
        self.emit(0xBE);
        self.modrm_rr(dst, src);
    }

    /// `LEA dst, [base + disp]`.
    pub fn lea(&mut self, size: OpSize, dst: Reg, base: Reg, disp: i32) {
        self.maybe_rex_rm(size, dst, base);
        self.emit(0x8D);
        self.modrm_mem(dst, base, disp);
    }

    /// `XCHG reg1, reg2`.
    pub fn xchg_rr(&mut self, size: OpSize, a: Reg, b: Reg) {
        self.maybe_size_prefix(size);
        self.maybe_rex_rr(size, a, b);
        self.emit(0x87);
        self.modrm_rr(a, b);
    }

    /// `PUSH reg` (64-bit).
    pub fn push(&mut self, reg: Reg) {
        if reg.is_ext() {
            self.emit_rex(false, false, false, true);
        }
        self.emit(0x50 + reg.lo3());
    }

    /// `POP reg` (64-bit).
    pub fn pop(&mut self, reg: Reg) {
        if reg.is_ext() {
            self.emit_rex(false, false, false, true);
        }
        self.emit(0x58 + reg.lo3());
    }

    /// `CDQ` — sign-extend EAX into EDX:EAX.
    pub fn cdq(&mut self) {
        self.emit(0x99);
    }

    /// `CQO` — sign-extend RAX into RDX:RAX.
    pub fn cqo(&mut self) {
        self.emit_rex(true, false, false, false);
        self.emit(0x99);
    }

    // ════════════════════════════════════════════════════════════════════
    // Arithmetic / Logic — register-register
    // ════════════════════════════════════════════════════════════════════

    /// Generic ALU reg, reg (ADD/OR/ADC/SBB/AND/SUB/XOR/CMP).
    fn alu_rr(&mut self, opcode: u8, size: OpSize, dst: Reg, src: Reg) {
        self.maybe_size_prefix(size);
        self.maybe_rex_rr(size, src, dst);
        self.emit(opcode);
        self.modrm_rr(src, dst);
    }

    /// `ADD dst, src`.
    pub fn add_rr(&mut self, size: OpSize, dst: Reg, src: Reg) {
        self.alu_rr(0x01, size, dst, src);
    }

    /// `OR dst, src`.
    pub fn or_rr(&mut self, size: OpSize, dst: Reg, src: Reg) {
        self.alu_rr(0x09, size, dst, src);
    }

    /// `ADC dst, src`.
    pub fn adc_rr(&mut self, size: OpSize, dst: Reg, src: Reg) {
        self.alu_rr(0x11, size, dst, src);
    }

    /// `SBB dst, src`.
    pub fn sbb_rr(&mut self, size: OpSize, dst: Reg, src: Reg) {
        self.alu_rr(0x19, size, dst, src);
    }

    /// `AND dst, src`.
    pub fn and_rr(&mut self, size: OpSize, dst: Reg, src: Reg) {
        self.alu_rr(0x21, size, dst, src);
    }

    /// `SUB dst, src`.
    pub fn sub_rr(&mut self, size: OpSize, dst: Reg, src: Reg) {
        self.alu_rr(0x29, size, dst, src);
    }

    /// `XOR dst, src`.
    pub fn xor_rr(&mut self, size: OpSize, dst: Reg, src: Reg) {
        self.alu_rr(0x31, size, dst, src);
    }

    /// `CMP a, b`.
    pub fn cmp_rr(&mut self, size: OpSize, a: Reg, b: Reg) {
        self.alu_rr(0x39, size, a, b);
    }

    /// `CMP reg, [base + disp]` (register compared with memory).
    pub fn cmp_rm(&mut self, size: OpSize, reg: Reg, base: Reg, disp: i32) {
        self.maybe_size_prefix(size);
        match size {
            OpSize::S8 => {
                self.maybe_rex_rm(OpSize::S8, reg, base);
                self.emit(0x3A);
            }
            _ => {
                self.maybe_rex_rm(size, reg, base);
                self.emit(0x3B);
            }
        }
        self.modrm_mem(reg, base, disp);
    }

    /// `TEST a, b`.
    pub fn test_rr(&mut self, size: OpSize, a: Reg, b: Reg) {
        self.maybe_size_prefix(size);
        self.maybe_rex_rr(size, b, a);
        if size == OpSize::S8 { self.emit(0x84); } else { self.emit(0x85); }
        self.modrm_rr(b, a);
    }

    // ── Arithmetic / Logic — register, immediate ───────────────────────

    /// Generic ALU reg, imm32 (Group 1: /digit = opcode selector).
    fn alu_ri(&mut self, digit: u8, size: OpSize, dst: Reg, imm: i32) {
        self.maybe_size_prefix(size);
        self.maybe_rex_single(size, dst);
        // Use short form (imm8 sign-extended) if possible.
        if imm >= -128 && imm <= 127 && size != OpSize::S8 {
            self.emit(0x83);
            self.modrm_rr(unsafe { core::mem::transmute::<u8, Reg>(digit) }, dst);
            self.emit(imm as u8);
        } else if size == OpSize::S8 {
            self.emit(0x80);
            self.modrm_rr(unsafe { core::mem::transmute::<u8, Reg>(digit) }, dst);
            self.emit(imm as u8);
        } else {
            self.emit(0x81);
            self.modrm_rr(unsafe { core::mem::transmute::<u8, Reg>(digit) }, dst);
            if size == OpSize::S16 {
                self.code.extend_from_slice(&(imm as i16).to_le_bytes());
            } else {
                self.emit_i32_le(imm);
            }
        }
    }

    /// `ADD dst, imm`.
    pub fn add_ri(&mut self, size: OpSize, dst: Reg, imm: i32) {
        self.alu_ri(0, size, dst, imm);
    }

    /// `OR dst, imm`.
    pub fn or_ri(&mut self, size: OpSize, dst: Reg, imm: i32) {
        self.alu_ri(1, size, dst, imm);
    }

    /// `ADC dst, imm`.
    pub fn adc_ri(&mut self, size: OpSize, dst: Reg, imm: i32) {
        self.alu_ri(2, size, dst, imm);
    }

    /// `AND dst, imm`.
    pub fn and_ri(&mut self, size: OpSize, dst: Reg, imm: i32) {
        self.alu_ri(4, size, dst, imm);
    }

    /// `SUB dst, imm`.
    pub fn sub_ri(&mut self, size: OpSize, dst: Reg, imm: i32) {
        self.alu_ri(5, size, dst, imm);
    }

    /// `XOR dst, imm`.
    pub fn xor_ri(&mut self, size: OpSize, dst: Reg, imm: i32) {
        self.alu_ri(6, size, dst, imm);
    }

    /// `CMP dst, imm`.
    pub fn cmp_ri(&mut self, size: OpSize, dst: Reg, imm: i32) {
        self.alu_ri(7, size, dst, imm);
    }

    /// `TEST reg, imm32`.
    pub fn test_ri(&mut self, size: OpSize, reg: Reg, imm: i32) {
        self.maybe_size_prefix(size);
        self.maybe_rex_single(size, reg);
        if size == OpSize::S8 {
            self.emit(0xF6);
        } else {
            self.emit(0xF7);
        }
        self.modrm_rr(Reg::Rax, reg); // /0
        if size == OpSize::S8 {
            self.emit(imm as u8);
        } else if size == OpSize::S16 {
            self.code.extend_from_slice(&(imm as i16).to_le_bytes());
        } else {
            self.emit_i32_le(imm);
        }
    }

    // ── Single-operand arithmetic ──────────────────────────────────────

    /// `INC reg` (preserves CF).
    pub fn inc(&mut self, size: OpSize, reg: Reg) {
        self.maybe_size_prefix(size);
        self.maybe_rex_single(size, reg);
        if size == OpSize::S8 {
            self.emit(0xFE);
        } else {
            self.emit(0xFF);
        }
        self.modrm_rr(Reg::Rax, reg); // /0
    }

    /// `DEC reg` (preserves CF).
    pub fn dec(&mut self, size: OpSize, reg: Reg) {
        self.maybe_size_prefix(size);
        self.maybe_rex_single(size, reg);
        if size == OpSize::S8 {
            self.emit(0xFE);
        } else {
            self.emit(0xFF);
        }
        self.modrm_rr(Reg::Rcx, reg); // /1
    }

    /// `NEG reg`.
    pub fn neg(&mut self, size: OpSize, reg: Reg) {
        self.maybe_size_prefix(size);
        self.maybe_rex_single(size, reg);
        if size == OpSize::S8 {
            self.emit(0xF6);
        } else {
            self.emit(0xF7);
        }
        self.modrm_rr(Reg::Rbx, reg); // /3
    }

    /// `NOT reg`.
    pub fn not(&mut self, size: OpSize, reg: Reg) {
        self.maybe_size_prefix(size);
        self.maybe_rex_single(size, reg);
        if size == OpSize::S8 {
            self.emit(0xF6);
        } else {
            self.emit(0xF7);
        }
        self.modrm_rr(Reg::Rdx, reg); // /2
    }

    /// `IMUL dst, src` (two-operand form, signed multiply).
    pub fn imul_rr(&mut self, size: OpSize, dst: Reg, src: Reg) {
        self.maybe_size_prefix(size);
        self.maybe_rex_rr(size, dst, src);
        self.emit(0x0F);
        self.emit(0xAF);
        self.modrm_rr(dst, src);
    }

    /// `IMUL dst, src, imm` (three-operand signed multiply).
    pub fn imul_rri(&mut self, size: OpSize, dst: Reg, src: Reg, imm: i32) {
        self.maybe_size_prefix(size);
        self.maybe_rex_rr(size, dst, src);
        if imm >= -128 && imm <= 127 {
            self.emit(0x6B);
            self.modrm_rr(dst, src);
            self.emit(imm as u8);
        } else {
            self.emit(0x69);
            self.modrm_rr(dst, src);
            self.emit_i32_le(imm);
        }
    }

    // ── Shifts ─────────────────────────────────────────────────────────

    /// Shift/rotate by CL: SHL/SHR/SAR/ROL/ROR/RCL/RCR.
    fn shift_cl(&mut self, digit: u8, size: OpSize, reg: Reg) {
        self.maybe_size_prefix(size);
        self.maybe_rex_single(size, reg);
        if size == OpSize::S8 {
            self.emit(0xD2);
        } else {
            self.emit(0xD3);
        }
        self.modrm_rr(unsafe { core::mem::transmute::<u8, Reg>(digit) }, reg);
    }

    /// Shift/rotate by immediate count.
    fn shift_imm(&mut self, digit: u8, size: OpSize, reg: Reg, count: u8) {
        self.maybe_size_prefix(size);
        self.maybe_rex_single(size, reg);
        if count == 1 {
            if size == OpSize::S8 {
                self.emit(0xD0);
            } else {
                self.emit(0xD1);
            }
            self.modrm_rr(unsafe { core::mem::transmute::<u8, Reg>(digit) }, reg);
        } else {
            if size == OpSize::S8 {
                self.emit(0xC0);
            } else {
                self.emit(0xC1);
            }
            self.modrm_rr(unsafe { core::mem::transmute::<u8, Reg>(digit) }, reg);
            self.emit(count);
        }
    }

    /// `SHL reg, CL`.
    pub fn shl_cl(&mut self, size: OpSize, reg: Reg) { self.shift_cl(4, size, reg); }
    /// `SHR reg, CL`.
    pub fn shr_cl(&mut self, size: OpSize, reg: Reg) { self.shift_cl(5, size, reg); }
    /// `SAR reg, CL`.
    pub fn sar_cl(&mut self, size: OpSize, reg: Reg) { self.shift_cl(7, size, reg); }
    /// `ROL reg, CL`.
    pub fn rol_cl(&mut self, size: OpSize, reg: Reg) { self.shift_cl(0, size, reg); }
    /// `ROR reg, CL`.
    pub fn ror_cl(&mut self, size: OpSize, reg: Reg) { self.shift_cl(1, size, reg); }

    /// `SHL reg, imm`.
    pub fn shl_ri(&mut self, size: OpSize, reg: Reg, count: u8) { self.shift_imm(4, size, reg, count); }
    /// `SHR reg, imm`.
    pub fn shr_ri(&mut self, size: OpSize, reg: Reg, count: u8) { self.shift_imm(5, size, reg, count); }
    /// `SAR reg, imm`.
    pub fn sar_ri(&mut self, size: OpSize, reg: Reg, count: u8) { self.shift_imm(7, size, reg, count); }

    // ════════════════════════════════════════════════════════════════════
    // Control Flow
    // ════════════════════════════════════════════════════════════════════

    /// `JMP rel32` to a label (forward or backward).
    pub fn jmp_label(&mut self, target: Label) {
        self.emit(0xE9);
        let patch = self.code.len();
        self.emit_i32_le(0); // placeholder
        let next = self.code.len();
        self.fixups.push(Fixup {
            label: target,
            patch_offset: patch,
            next_ip_offset: next,
        });
    }

    /// `Jcc rel32` — conditional jump to a label.
    pub fn jcc_label(&mut self, cc: Cc, target: Label) {
        self.emit(0x0F);
        self.emit(0x80 + cc as u8);
        let patch = self.code.len();
        self.emit_i32_le(0); // placeholder
        let next = self.code.len();
        self.fixups.push(Fixup {
            label: target,
            patch_offset: patch,
            next_ip_offset: next,
        });
    }

    /// `JMP reg` (indirect jump through register).
    pub fn jmp_reg(&mut self, reg: Reg) {
        if reg.is_ext() {
            self.emit_rex(false, false, false, true);
        }
        self.emit(0xFF);
        self.modrm_rr(Reg::Rsp, reg); // /4
    }

    /// `CALL rel32` — call with 32-bit relative offset (resolved from label).
    pub fn call_label(&mut self, target: Label) {
        self.emit(0xE8);
        let patch = self.code.len();
        self.emit_i32_le(0);
        let next = self.code.len();
        self.fixups.push(Fixup {
            label: target,
            patch_offset: patch,
            next_ip_offset: next,
        });
    }

    /// `CALL reg` (indirect call through register).
    pub fn call_reg(&mut self, reg: Reg) {
        if reg.is_ext() {
            self.emit_rex(false, false, false, true);
        }
        self.emit(0xFF);
        self.modrm_rr(Reg::Rdx, reg); // /2
    }

    /// `CALL [rip + rel32]` — call through absolute address loaded from memory.
    ///
    /// Emits: `mov rax, imm64; call rax` (10 + 2 = 12 bytes).
    /// Used to call runtime helper functions at known addresses.
    pub fn call_abs(&mut self, addr: u64) {
        // mov rax, addr
        self.mov_ri64(Reg::Rax, addr);
        // call rax
        self.call_reg(Reg::Rax);
    }

    /// `JMP abs` — jump to an absolute 64-bit address.
    ///
    /// Emits: `mov rax, imm64; jmp rax` (10 + 2 = 12 bytes).
    pub fn jmp_abs(&mut self, addr: u64) {
        // mov rax, addr
        self.mov_ri64(Reg::Rax, addr);
        // jmp rax
        self.jmp_reg(Reg::Rax);
    }

    /// `RET`.
    pub fn ret(&mut self) {
        self.emit(0xC3);
    }

    /// `NOP`.
    pub fn nop(&mut self) {
        self.emit(0x90);
    }

    /// `INT3` — breakpoint.
    pub fn int3(&mut self) {
        self.emit(0xCC);
    }

    // ════════════════════════════════════════════════════════════════════
    // Conditional Set / Move
    // ════════════════════════════════════════════════════════════════════

    /// `SETcc reg8` — set byte register to 0 or 1 based on condition.
    pub fn setcc(&mut self, cc: Cc, reg: Reg) {
        if reg.is_ext() {
            self.emit_rex(false, false, false, true);
        }
        self.emit(0x0F);
        self.emit(0x90 + cc as u8);
        self.modrm_rr(Reg::Rax, reg); // /0
    }

    /// `CMOVcc dst, src` — conditional move.
    pub fn cmovcc(&mut self, cc: Cc, size: OpSize, dst: Reg, src: Reg) {
        self.maybe_size_prefix(size);
        self.maybe_rex_rr(size, dst, src);
        self.emit(0x0F);
        self.emit(0x40 + cc as u8);
        self.modrm_rr(dst, src);
    }

    // ════════════════════════════════════════════════════════════════════
    // Flags
    // ════════════════════════════════════════════════════════════════════

    /// `PUSHFQ` — push RFLAGS onto stack.
    pub fn pushfq(&mut self) {
        self.emit(0x9C);
    }

    /// `POPFQ` — pop RFLAGS from stack.
    pub fn popfq(&mut self) {
        self.emit(0x9D);
    }

    /// `LAHF` — load AH from lower 8 bits of flags (SF:ZF:0:AF:0:PF:1:CF).
    pub fn lahf(&mut self) {
        self.emit(0x9F);
    }

    /// `SAHF` — store AH into lower 8 bits of flags.
    pub fn sahf(&mut self) {
        self.emit(0x9E);
    }

    /// `CLC` — clear carry flag.
    pub fn clc(&mut self) {
        self.emit(0xF8);
    }

    /// `STC` — set carry flag.
    pub fn stc(&mut self) {
        self.emit(0xF9);
    }

    /// `CLD` — clear direction flag.
    pub fn cld(&mut self) {
        self.emit(0xFC);
    }
}
