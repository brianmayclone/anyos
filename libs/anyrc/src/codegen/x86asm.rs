use crate::prelude::*;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reg {
    RAX,
    RCX,
    RDX,
    RBX,
    RSP,
    RBP,
    RSI,
    RDI,
    R8,
    R9,
    R10,
    R11,
    R12,
    R13,
    R14,
    R15,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CondCode {
    Equal,
    NotEqual,
    Less,
    LessEqual,
    Greater,
    GreaterEqual,
    Below,
    BelowEqual,
    Above,
    AboveEqual,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Label(pub usize);

struct Fixup {
    offset: usize,
    label: Label,
    kind: FixupKind,
}

enum FixupKind {
    Rel32,
}

pub struct Relocation {
    pub offset: usize,
    pub symbol: String,
    pub kind: RelocKind,
}

pub enum RelocKind {
    PcRel32,
    Abs64,
}

pub struct X86Assembler {
    code: Vec<u8>,
    labels: Vec<Option<usize>>,
    fixups: Vec<Fixup>,
    pub relocations: Vec<Relocation>,
}

impl X86Assembler {
    pub fn new() -> Self {
        X86Assembler {
            code: Vec::new(),
            labels: Vec::new(),
            fixups: Vec::new(),
            relocations: Vec::new(),
        }
    }

    pub fn code(&self) -> &[u8] {
        &self.code
    }

    pub fn new_label(&mut self) -> Label {
        let id = self.labels.len();
        self.labels.push(None);
        Label(id)
    }

    pub fn bind_label(&mut self, label: Label) {
        self.labels[label.0] = Some(self.code.len());
    }

    pub fn resolve_fixups(&mut self) {
        for fixup in &self.fixups {
            let target = self.labels[fixup.label.0].expect("unresolved label");
            match fixup.kind {
                FixupKind::Rel32 => {
                    // rel32 is relative to the end of the instruction (offset + 4)
                    let rel = (target as i64) - ((fixup.offset + 4) as i64);
                    let bytes = (rel as i32).to_le_bytes();
                    self.code[fixup.offset..fixup.offset + 4].copy_from_slice(&bytes);
                }
            }
        }
    }

    fn reg_code(reg: Reg) -> u8 {
        match reg {
            Reg::RAX => 0,
            Reg::RCX => 1,
            Reg::RDX => 2,
            Reg::RBX => 3,
            Reg::RSP => 4,
            Reg::RBP => 5,
            Reg::RSI => 6,
            Reg::RDI => 7,
            Reg::R8 => 0,
            Reg::R9 => 1,
            Reg::R10 => 2,
            Reg::R11 => 3,
            Reg::R12 => 4,
            Reg::R13 => 5,
            Reg::R14 => 6,
            Reg::R15 => 7,
        }
    }

    fn is_extended(reg: Reg) -> bool {
        matches!(
            reg,
            Reg::R8 | Reg::R9 | Reg::R10 | Reg::R11 | Reg::R12 | Reg::R13 | Reg::R14 | Reg::R15
        )
    }

    /// Emit REX prefix. `reg` is the ModRM.reg field register, `rm` is the ModRM.rm register.
    fn rex(&mut self, w: bool, reg: Reg, rm: Reg) {
        let byte = 0x40
            | if w { 0x08 } else { 0 }
            | if Self::is_extended(reg) { 0x04 } else { 0 }  // REX.R
            | if Self::is_extended(rm) { 0x01 } else { 0 }; // REX.B
        self.code.push(byte);
    }

    /// Emit REX for single-operand instructions (reg field is opcode extension).
    fn rex_single(&mut self, w: bool, rm: Reg) {
        let byte = 0x40 | if w { 0x08 } else { 0 } | if Self::is_extended(rm) { 0x01 } else { 0 };
        self.code.push(byte);
    }

    /// Emit REX where `reg` is extended (R bit) but rm is an opcode extension.
    fn rex_r(&mut self, w: bool, reg: Reg) {
        let byte = 0x40 | if w { 0x08 } else { 0 } | if Self::is_extended(reg) { 0x04 } else { 0 };
        self.code.push(byte);
    }

    fn modrm(mode: u8, reg: u8, rm: u8) -> u8 {
        (mode << 6) | (reg << 3) | rm
    }

    /// Emit ModRM + optional SIB + displacement for memory addressing [base+disp].
    fn emit_mem_modrm(&mut self, reg_field: u8, base: Reg, disp: i32) {
        let base_code = Self::reg_code(base);
        let needs_sib = base_code == 4; // RSP/R12

        if disp == 0 && base_code != 5 {
            // mod=00 (no disp), except RBP/R13 which needs mod=01
            self.code.push(Self::modrm(0b00, reg_field, base_code));
            if needs_sib {
                self.code.push(0x24); // SIB: index=none, base=RSP
            }
        } else if disp >= -128 && disp <= 127 {
            // mod=01 (disp8)
            self.code.push(Self::modrm(0b01, reg_field, base_code));
            if needs_sib {
                self.code.push(0x24);
            }
            self.code.push(disp as u8);
        } else {
            // mod=10 (disp32)
            self.code.push(Self::modrm(0b10, reg_field, base_code));
            if needs_sib {
                self.code.push(0x24);
            }
            self.code.extend_from_slice(&(disp).to_le_bytes());
        }
    }

    // ── Instructions ──

    pub fn mov_rr(&mut self, dst: Reg, src: Reg) {
        // MOV r/m64, r64: REX.W 89 /r (src is reg field, dst is rm field)
        self.rex(true, src, dst);
        self.code.push(0x89);
        self.code
            .push(Self::modrm(0b11, Self::reg_code(src), Self::reg_code(dst)));
    }

    pub fn mov_ri(&mut self, dst: Reg, imm: i64) {
        // MOV r64, imm64: REX.W B8+rd imm64
        self.rex_single(true, dst);
        self.code.push(0xB8 + Self::reg_code(dst));
        self.code.extend_from_slice(&imm.to_le_bytes());
    }

    pub fn mov_rm(&mut self, dst: Reg, base: Reg, disp: i32) {
        // MOV r64, [base+disp]: REX.W 8B /r
        self.rex(true, dst, base);
        self.code.push(0x8B);
        self.emit_mem_modrm(Self::reg_code(dst), base, disp);
    }

    pub fn movzx_rm_sized(&mut self, dst: Reg, base: Reg, disp: i32, size: i32) {
        match size {
            1 => {
                // MOVZX r64, r/m8: REX.W 0F B6 /r
                self.rex(true, dst, base);
                self.code.push(0x0F);
                self.code.push(0xB6);
                self.emit_mem_modrm(Self::reg_code(dst), base, disp);
            }
            2 => {
                // MOVZX r64, r/m16: REX.W 0F B7 /r
                self.rex(true, dst, base);
                self.code.push(0x0F);
                self.code.push(0xB7);
                self.emit_mem_modrm(Self::reg_code(dst), base, disp);
            }
            4 => {
                // MOV r32, r/m32 zero-extends into the 64-bit destination.
                self.rex(false, dst, base);
                self.code.push(0x8B);
                self.emit_mem_modrm(Self::reg_code(dst), base, disp);
            }
            _ => self.mov_rm(dst, base, disp),
        }
    }

    pub fn mov_mr(&mut self, base: Reg, disp: i32, src: Reg) {
        // MOV [base+disp], r64: REX.W 89 /r
        self.rex(true, src, base);
        self.code.push(0x89);
        self.emit_mem_modrm(Self::reg_code(src), base, disp);
    }

    pub fn mov_mr_sized(&mut self, base: Reg, disp: i32, src: Reg, size: i32) {
        match size {
            1 => {
                // MOV r/m8, r8: 88 /r
                self.rex(false, src, base);
                self.code.push(0x88);
                self.emit_mem_modrm(Self::reg_code(src), base, disp);
            }
            2 => {
                // MOV r/m16, r16: 66 89 /r
                self.code.push(0x66);
                self.rex(false, src, base);
                self.code.push(0x89);
                self.emit_mem_modrm(Self::reg_code(src), base, disp);
            }
            4 => {
                // MOV r/m32, r32: 89 /r
                self.rex(false, src, base);
                self.code.push(0x89);
                self.emit_mem_modrm(Self::reg_code(src), base, disp);
            }
            _ => self.mov_mr(base, disp, src),
        }
    }

    pub fn mov_mi(&mut self, base: Reg, disp: i32, imm: i32) {
        // MOV [base+disp], imm32: REX.W C7 /0 imm32
        self.rex_single(true, base);
        self.code.push(0xC7);
        self.emit_mem_modrm(0, base, disp);
        self.code.extend_from_slice(&imm.to_le_bytes());
    }

    pub fn movsx_r32_r64(&mut self, dst: Reg, src: Reg) {
        // MOVSXD r64, r/m32: REX.W 63 /r
        self.rex(true, dst, src);
        self.code.push(0x63);
        self.code
            .push(Self::modrm(0b11, Self::reg_code(dst), Self::reg_code(src)));
    }

    fn alu_rr(&mut self, opcode: u8, dst: Reg, src: Reg) {
        self.rex(true, src, dst);
        self.code.push(opcode);
        self.code
            .push(Self::modrm(0b11, Self::reg_code(src), Self::reg_code(dst)));
    }

    fn alu_ri(&mut self, ext: u8, dst: Reg, imm: i32) {
        self.rex_single(true, dst);
        self.code.push(0x81);
        self.code.push(Self::modrm(0b11, ext, Self::reg_code(dst)));
        self.code.extend_from_slice(&imm.to_le_bytes());
    }

    pub fn add_rr(&mut self, dst: Reg, src: Reg) {
        self.alu_rr(0x01, dst, src);
    }

    pub fn add_ri(&mut self, dst: Reg, imm: i32) {
        self.alu_ri(0, dst, imm);
    }

    pub fn sub_rr(&mut self, dst: Reg, src: Reg) {
        self.alu_rr(0x29, dst, src);
    }

    pub fn sub_ri(&mut self, dst: Reg, imm: i32) {
        self.alu_ri(5, dst, imm);
    }

    pub fn imul_rr(&mut self, dst: Reg, src: Reg) {
        // IMUL r64, r/m64: REX.W 0F AF /r (dst is reg, src is rm)
        self.rex(true, dst, src);
        self.code.push(0x0F);
        self.code.push(0xAF);
        self.code
            .push(Self::modrm(0b11, Self::reg_code(dst), Self::reg_code(src)));
    }

    pub fn imul_ri(&mut self, dst: Reg, imm: i64) {
        // IMUL r64, r/m64, imm32: REX.W 69 /r id
        self.rex(true, dst, dst);
        self.code.push(0x69);
        self.code
            .push(Self::modrm(0b11, Self::reg_code(dst), Self::reg_code(dst)));
        self.code.extend_from_slice(&(imm as i32).to_le_bytes());
    }

    pub fn cqo(&mut self) {
        // CQO: REX.W 99
        self.code.push(0x48);
        self.code.push(0x99);
    }

    pub fn idiv_r(&mut self, src: Reg) {
        // IDIV r/m64: REX.W F7 /7
        self.rex_single(true, src);
        self.code.push(0xF7);
        self.code.push(Self::modrm(0b11, 7, Self::reg_code(src)));
    }

    pub fn and_rr(&mut self, dst: Reg, src: Reg) {
        self.alu_rr(0x21, dst, src);
    }

    pub fn or_rr(&mut self, dst: Reg, src: Reg) {
        self.alu_rr(0x09, dst, src);
    }

    pub fn xor_rr(&mut self, dst: Reg, src: Reg) {
        self.alu_rr(0x31, dst, src);
    }

    pub fn xor_ri(&mut self, dst: Reg, imm: i32) {
        self.alu_ri(6, dst, imm);
    }

    pub fn shl_ri(&mut self, dst: Reg, imm: u8) {
        // SHL r/m64, imm8: REX.W C1 /4 imm8
        self.rex_single(true, dst);
        self.code.push(0xC1);
        self.code.push(Self::modrm(0b11, 4, Self::reg_code(dst)));
        self.code.push(imm);
    }

    pub fn shr_ri(&mut self, dst: Reg, imm: u8) {
        self.rex_single(true, dst);
        self.code.push(0xC1);
        self.code.push(Self::modrm(0b11, 5, Self::reg_code(dst)));
        self.code.push(imm);
    }

    pub fn shl_cl(&mut self, dst: Reg) {
        // SHL r/m64, CL: REX.W D3 /4
        self.rex_single(true, dst);
        self.code.push(0xD3);
        self.code.push(Self::modrm(0b11, 4, Self::reg_code(dst)));
    }

    pub fn shr_cl(&mut self, dst: Reg) {
        // SHR r/m64, CL: REX.W D3 /5
        self.rex_single(true, dst);
        self.code.push(0xD3);
        self.code.push(Self::modrm(0b11, 5, Self::reg_code(dst)));
    }

    pub fn sar_ri(&mut self, dst: Reg, imm: u8) {
        self.rex_single(true, dst);
        self.code.push(0xC1);
        self.code.push(Self::modrm(0b11, 7, Self::reg_code(dst)));
        self.code.push(imm);
    }

    pub fn neg_r(&mut self, reg: Reg) {
        // NEG r/m64: REX.W F7 /3
        self.rex_single(true, reg);
        self.code.push(0xF7);
        self.code.push(Self::modrm(0b11, 3, Self::reg_code(reg)));
    }

    pub fn not_r(&mut self, reg: Reg) {
        // NOT r/m64: REX.W F7 /2
        self.rex_single(true, reg);
        self.code.push(0xF7);
        self.code.push(Self::modrm(0b11, 2, Self::reg_code(reg)));
    }

    pub fn cmp_rr(&mut self, a: Reg, b: Reg) {
        // CMP r/m64, r64: REX.W 39 /r (b is reg field, a is rm)
        self.alu_rr(0x39, a, b);
    }

    pub fn cmp_ri(&mut self, reg: Reg, imm: i32) {
        self.alu_ri(7, reg, imm);
    }

    pub fn test_rr(&mut self, a: Reg, b: Reg) {
        // TEST r/m64, r64: REX.W 85 /r
        self.alu_rr(0x85, a, b);
    }

    fn cc_byte(cc: CondCode) -> u8 {
        match cc {
            CondCode::Equal => 0x04,        // JE/SETE
            CondCode::NotEqual => 0x05,     // JNE/SETNE
            CondCode::Less => 0x0C,         // JL/SETL
            CondCode::LessEqual => 0x0E,    // JLE/SETLE
            CondCode::Greater => 0x0F,      // JG/SETG
            CondCode::GreaterEqual => 0x0D, // JGE/SETGE
            CondCode::Below => 0x02,        // JB/SETB
            CondCode::BelowEqual => 0x06,   // JBE/SETBE
            CondCode::Above => 0x07,        // JA/SETA
            CondCode::AboveEqual => 0x03,   // JAE/SETAE
        }
    }

    pub fn setcc(&mut self, cc: CondCode, dst: Reg) {
        // SETcc r/m8: 0F 9x ModRM
        // Need REX if dst is R8-R15 (for REX.B), or if dst is SPL/BPL/SIL/DIL
        if Self::is_extended(dst) {
            self.code.push(0x41); // REX.B
        } else {
            // Need REX prefix to access SIL/DIL/BPL/SPL (otherwise we get AH/CH/DH/BH)
            // Only needed for RSP/RBP/RSI/RDI when used as byte registers
            let code = Self::reg_code(dst);
            if code >= 4 {
                self.code.push(0x40); // REX (no bits set, just the prefix)
            }
        }
        self.code.push(0x0F);
        self.code.push(0x90 + Self::cc_byte(cc));
        self.code.push(Self::modrm(0b11, 0, Self::reg_code(dst)));
    }

    pub fn movzx_r8_r64(&mut self, dst: Reg, src: Reg) {
        // MOVZX r64, r/m8: REX.W 0F B6 /r
        self.rex(true, dst, src);
        self.code.push(0x0F);
        self.code.push(0xB6);
        self.code
            .push(Self::modrm(0b11, Self::reg_code(dst), Self::reg_code(src)));
    }

    pub fn jmp(&mut self, label: Label) {
        // JMP rel32: E9 rel32
        self.code.push(0xE9);
        let offset = self.code.len();
        self.code.extend_from_slice(&[0; 4]);
        self.fixups.push(Fixup {
            offset,
            label,
            kind: FixupKind::Rel32,
        });
    }

    pub fn jcc(&mut self, cc: CondCode, label: Label) {
        // Jcc rel32: 0F 8x rel32
        self.code.push(0x0F);
        self.code.push(0x80 + Self::cc_byte(cc));
        let offset = self.code.len();
        self.code.extend_from_slice(&[0; 4]);
        self.fixups.push(Fixup {
            offset,
            label,
            kind: FixupKind::Rel32,
        });
    }

    pub fn call_rel(&mut self, label: Label) {
        // CALL rel32: E8 rel32
        self.code.push(0xE8);
        let offset = self.code.len();
        self.code.extend_from_slice(&[0; 4]);
        self.fixups.push(Fixup {
            offset,
            label,
            kind: FixupKind::Rel32,
        });
    }

    pub fn call_extern(&mut self, symbol: &str) {
        // CALL rel32 with relocation
        self.code.push(0xE8);
        let offset = self.code.len();
        self.code.extend_from_slice(&[0; 4]);
        self.relocations.push(Relocation {
            offset,
            symbol: symbol.to_string(),
            kind: RelocKind::PcRel32,
        });
    }

    /// CALL r64: FF /2 (indirect call through register)
    pub fn call_reg(&mut self, reg: Reg) {
        self.rex_single(true, reg);
        self.code.push(0xFF);
        self.code.push(Self::modrm(3, 2, Self::reg_code(reg)));
    }

    pub fn ret(&mut self) {
        self.code.push(0xC3);
    }

    pub fn push(&mut self, reg: Reg) {
        // PUSH r64: 50+rd (REX.B for extended regs)
        if Self::is_extended(reg) {
            self.code.push(0x41); // REX.B
        }
        self.code.push(0x50 + Self::reg_code(reg));
    }

    pub fn pop(&mut self, reg: Reg) {
        // POP r64: 58+rd
        if Self::is_extended(reg) {
            self.code.push(0x41);
        }
        self.code.push(0x58 + Self::reg_code(reg));
    }

    pub fn lea(&mut self, dst: Reg, base: Reg, disp: i32) {
        // LEA r64, [base+disp]: REX.W 8D /r
        self.rex(true, dst, base);
        self.code.push(0x8D);
        self.emit_mem_modrm(Self::reg_code(dst), base, disp);
    }

    /// LEA dst, [rip + symbol] — emits with a PC-relative relocation
    pub fn lea_rip_relative(&mut self, dst: Reg, symbol: &str) {
        // LEA r64, [rip+disp32]: REX.W 8D ModRM(00, reg, 101)
        // REX.W=1, REX.R = is_extended(dst), REX.B=0 (RIP has no base reg)
        let rex = 0x48 | if Self::is_extended(dst) { 0x04 } else { 0 };
        self.code.push(rex);
        self.code.push(0x8D);
        // ModRM: mod=00, reg=dst, rm=101 (RIP-relative)
        self.code
            .push(Self::modrm(0b00, Self::reg_code(dst), 0b101));
        let offset = self.code.len();
        self.code.extend_from_slice(&[0; 4]); // disp32 placeholder
        self.relocations.push(Relocation {
            offset,
            symbol: symbol.to_string(),
            kind: RelocKind::PcRel32,
        });
    }

    pub fn nop(&mut self) {
        self.code.push(0x90);
    }

    pub fn syscall(&mut self) {
        self.code.push(0x0F);
        self.code.push(0x05);
    }

    pub fn emit_raw(&mut self, bytes: &[u8]) {
        self.code.extend_from_slice(bytes);
    }
}
