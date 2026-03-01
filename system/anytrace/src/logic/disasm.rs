//! x86_64 instruction decoder (table-driven).
//!
//! Supports common instructions needed for debugging: MOV, PUSH, POP,
//! arithmetic, LEA, CALL, RET, JMP, Jcc, SYSCALL, INT3, NOP.

use alloc::string::String;

/// A decoded x86_64 instruction.
pub struct DecodedInstr {
    /// Length of the instruction in bytes.
    pub len: u8,
    /// Mnemonic (e.g., "mov", "call", "nop").
    pub mnemonic: [u8; 12],
    pub mnemonic_len: u8,
    /// Operand string (e.g., "rax, rbx" or "qword [rsp+0x10]").
    pub operands: [u8; 48],
    pub operands_len: u8,
    /// Raw instruction bytes.
    pub bytes: [u8; 15],
}

impl DecodedInstr {
    fn new() -> Self {
        Self {
            len: 0,
            mnemonic: [0; 12],
            mnemonic_len: 0,
            operands: [0; 48],
            operands_len: 0,
            bytes: [0; 15],
        }
    }

    /// Get mnemonic as str.
    pub fn mnemonic_str(&self) -> &str {
        core::str::from_utf8(&self.mnemonic[..self.mnemonic_len as usize]).unwrap_or("???")
    }

    /// Get operands as str.
    pub fn operands_str(&self) -> &str {
        core::str::from_utf8(&self.operands[..self.operands_len as usize]).unwrap_or("")
    }

    fn set_mnemonic(&mut self, s: &str) {
        let bytes = s.as_bytes();
        let len = bytes.len().min(12);
        self.mnemonic[..len].copy_from_slice(&bytes[..len]);
        self.mnemonic_len = len as u8;
    }

    fn set_operands(&mut self, s: &str) {
        let bytes = s.as_bytes();
        let len = bytes.len().min(48);
        self.operands[..len].copy_from_slice(&bytes[..len]);
        self.operands_len = len as u8;
    }
}

const REG64: [&str; 16] = [
    "rax", "rcx", "rdx", "rbx", "rsp", "rbp", "rsi", "rdi",
    "r8", "r9", "r10", "r11", "r12", "r13", "r14", "r15",
];

const REG32: [&str; 16] = [
    "eax", "ecx", "edx", "ebx", "esp", "ebp", "esi", "edi",
    "r8d", "r9d", "r10d", "r11d", "r12d", "r13d", "r14d", "r15d",
];

const REG8: [&str; 16] = [
    "al", "cl", "dl", "bl", "spl", "bpl", "sil", "dil",
    "r8b", "r9b", "r10b", "r11b", "r12b", "r13b", "r14b", "r15b",
];

const CC_NAMES: [&str; 16] = [
    "o", "no", "b", "ae", "e", "ne", "be", "a",
    "s", "ns", "p", "np", "l", "ge", "le", "g",
];

/// Decode one instruction from `code` at the given `rip`.
///
/// Returns `None` if the bytes cannot be decoded.
pub fn decode(code: &[u8], rip: u64) -> Option<DecodedInstr> {
    if code.is_empty() {
        return None;
    }

    let mut instr = DecodedInstr::new();
    let mut pos = 0;
    let mut rex: u8 = 0;
    let mut has_rex = false;
    let mut op_size_override = false;

    // Parse prefixes
    loop {
        if pos >= code.len() {
            return None;
        }
        match code[pos] {
            0x66 => { op_size_override = true; pos += 1; }
            0xF0 | 0xF2 | 0xF3 | 0x26 | 0x2E | 0x36 | 0x3E | 0x64 | 0x65 => { pos += 1; }
            0x40..=0x4F => {
                rex = code[pos];
                has_rex = true;
                pos += 1;
            }
            _ => break,
        }
    }

    if pos >= code.len() {
        return None;
    }

    let rex_w = has_rex && (rex & 0x08) != 0;
    let rex_r = has_rex && (rex & 0x04) != 0;
    let rex_b = has_rex && (rex & 0x01) != 0;

    let opcode = code[pos];
    pos += 1;

    match opcode {
        // NOP
        0x90 => {
            instr.set_mnemonic("nop");
        }

        // RET
        0xC3 => {
            instr.set_mnemonic("ret");
        }

        // INT3
        0xCC => {
            instr.set_mnemonic("int3");
        }

        // INT imm8
        0xCD => {
            if pos >= code.len() { return None; }
            let imm = code[pos];
            pos += 1;
            instr.set_mnemonic("int");
            instr.set_operands(&alloc::format!("{:#x}", imm));
        }

        // SYSCALL (0x0F 0x05)
        0x0F => {
            if pos >= code.len() { return None; }
            let op2 = code[pos];
            pos += 1;
            match op2 {
                0x05 => instr.set_mnemonic("syscall"),
                0x1F => {
                    // NOP with ModR/M (multi-byte NOP)
                    instr.set_mnemonic("nop");
                    if pos < code.len() {
                        let modrm = code[pos];
                        pos += 1;
                        let md = modrm >> 6;
                        let rm = modrm & 7;
                        // Skip SIB + displacement
                        if md != 3 && rm == 4 && pos < code.len() { pos += 1; }
                        match md {
                            1 => if pos < code.len() { pos += 1; },
                            2 => if pos + 4 <= code.len() { pos += 4; },
                            _ => {}
                        }
                    }
                }
                // Jcc rel32 (0x0F 0x80..0x8F)
                0x80..=0x8F => {
                    if pos + 4 > code.len() { return None; }
                    let rel = i32::from_le_bytes([code[pos], code[pos+1], code[pos+2], code[pos+3]]);
                    pos += 4;
                    let target = (rip as i64 + pos as i64 + rel as i64) as u64;
                    let cc = (op2 - 0x80) as usize;
                    instr.set_mnemonic(&alloc::format!("j{}", CC_NAMES[cc]));
                    instr.set_operands(&alloc::format!("{:#x}", target));
                }
                // MOVZX r, r/m8 (0x0F 0xB6)
                0xB6 => {
                    instr.set_mnemonic("movzx");
                    if pos < code.len() {
                        let (ops, consumed) = decode_modrm_reg(code, pos, rex_r, rex_b, rex_w, true, false);
                        pos += consumed;
                        instr.set_operands(&ops);
                    }
                }
                // MOVZX r, r/m16 (0x0F 0xB7)
                0xB7 => {
                    instr.set_mnemonic("movzx");
                    if pos < code.len() {
                        let (ops, consumed) = decode_modrm_reg(code, pos, rex_r, rex_b, rex_w, true, false);
                        pos += consumed;
                        instr.set_operands(&ops);
                    }
                }
                // IMUL r, r/m (0x0F 0xAF)
                0xAF => {
                    instr.set_mnemonic("imul");
                    if pos < code.len() {
                        let (ops, consumed) = decode_modrm_reg(code, pos, rex_r, rex_b, rex_w, true, false);
                        pos += consumed;
                        instr.set_operands(&ops);
                    }
                }
                // ENDBR64 (0x0F 0x1E 0xFA)
                0x1E => {
                    if pos < code.len() && code[pos] == 0xFA {
                        pos += 1;
                        instr.set_mnemonic("endbr64");
                    } else {
                        instr.set_mnemonic("nop");
                        if pos < code.len() { pos += 1; }
                    }
                }
                _ => {
                    instr.set_mnemonic("db");
                    instr.set_operands(&alloc::format!("0x0f, {:#04x}", op2));
                }
            }
        }

        // PUSH r64 (0x50+r)
        0x50..=0x57 => {
            let reg = ((opcode - 0x50) | if rex_b { 8 } else { 0 }) as usize;
            instr.set_mnemonic("push");
            instr.set_operands(REG64[reg]);
        }

        // POP r64 (0x58+r)
        0x58..=0x5F => {
            let reg = ((opcode - 0x58) | if rex_b { 8 } else { 0 }) as usize;
            instr.set_mnemonic("pop");
            instr.set_operands(REG64[reg]);
        }

        // MOV r64, imm64 (0xB8+r) / MOV r32, imm32 (without REX.W)
        0xB8..=0xBF => {
            let reg = ((opcode - 0xB8) | if rex_b { 8 } else { 0 }) as usize;
            if rex_w {
                if pos + 8 > code.len() { return None; }
                let imm = u64::from_le_bytes([
                    code[pos], code[pos+1], code[pos+2], code[pos+3],
                    code[pos+4], code[pos+5], code[pos+6], code[pos+7],
                ]);
                pos += 8;
                instr.set_mnemonic("mov");
                instr.set_operands(&alloc::format!("{}, {:#x}", REG64[reg], imm));
            } else {
                if pos + 4 > code.len() { return None; }
                let imm = u32::from_le_bytes([code[pos], code[pos+1], code[pos+2], code[pos+3]]);
                pos += 4;
                instr.set_mnemonic("mov");
                instr.set_operands(&alloc::format!("{}, {:#x}", REG32[reg], imm));
            }
        }

        // CALL rel32
        0xE8 => {
            if pos + 4 > code.len() { return None; }
            let rel = i32::from_le_bytes([code[pos], code[pos+1], code[pos+2], code[pos+3]]);
            pos += 4;
            let target = (rip as i64 + pos as i64 + rel as i64) as u64;
            instr.set_mnemonic("call");
            instr.set_operands(&alloc::format!("{:#x}", target));
        }

        // JMP rel32
        0xE9 => {
            if pos + 4 > code.len() { return None; }
            let rel = i32::from_le_bytes([code[pos], code[pos+1], code[pos+2], code[pos+3]]);
            pos += 4;
            let target = (rip as i64 + pos as i64 + rel as i64) as u64;
            instr.set_mnemonic("jmp");
            instr.set_operands(&alloc::format!("{:#x}", target));
        }

        // JMP rel8
        0xEB => {
            if pos >= code.len() { return None; }
            let rel = code[pos] as i8;
            pos += 1;
            let target = (rip as i64 + pos as i64 + rel as i64) as u64;
            instr.set_mnemonic("jmp");
            instr.set_operands(&alloc::format!("{:#x}", target));
        }

        // Jcc rel8 (0x70..0x7F)
        0x70..=0x7F => {
            if pos >= code.len() { return None; }
            let rel = code[pos] as i8;
            pos += 1;
            let target = (rip as i64 + pos as i64 + rel as i64) as u64;
            let cc = (opcode - 0x70) as usize;
            instr.set_mnemonic(&alloc::format!("j{}", CC_NAMES[cc]));
            instr.set_operands(&alloc::format!("{:#x}", target));
        }

        // LEA r, [m] (0x8D)
        0x8D => {
            instr.set_mnemonic("lea");
            if pos < code.len() {
                let (ops, consumed) = decode_modrm_reg(code, pos, rex_r, rex_b, rex_w, true, false);
                pos += consumed;
                instr.set_operands(&ops);
            }
        }

        // MOV r/m, r (0x89) / MOV r, r/m (0x8B)
        0x89 => {
            instr.set_mnemonic("mov");
            if pos < code.len() {
                let (ops, consumed) = decode_modrm_reg(code, pos, rex_r, rex_b, rex_w, false, false);
                pos += consumed;
                instr.set_operands(&ops);
            }
        }
        0x8B => {
            instr.set_mnemonic("mov");
            if pos < code.len() {
                let (ops, consumed) = decode_modrm_reg(code, pos, rex_r, rex_b, rex_w, true, false);
                pos += consumed;
                instr.set_operands(&ops);
            }
        }

        // ADD/OR/ADC/SBB/AND/SUB/XOR/CMP r/m, imm8 (0x83)
        0x83 => {
            if pos >= code.len() { return None; }
            let modrm = code[pos];
            let op_ext = (modrm >> 3) & 7;
            let mnemonic = match op_ext {
                0 => "add", 1 => "or", 2 => "adc", 3 => "sbb",
                4 => "and", 5 => "sub", 6 => "xor", 7 => "cmp",
                _ => "???",
            };
            instr.set_mnemonic(mnemonic);
            let (rm_str, consumed) = decode_modrm_rm(code, pos, rex_b, rex_w);
            pos += consumed;
            if pos >= code.len() { return None; }
            let imm = code[pos] as i8;
            pos += 1;
            instr.set_operands(&alloc::format!("{}, {:#x}", rm_str, imm));
        }

        // ADD/OR/ADC/SBB/AND/SUB/XOR/CMP r/m, imm32 (0x81)
        0x81 => {
            if pos >= code.len() { return None; }
            let modrm = code[pos];
            let op_ext = (modrm >> 3) & 7;
            let mnemonic = match op_ext {
                0 => "add", 1 => "or", 2 => "adc", 3 => "sbb",
                4 => "and", 5 => "sub", 6 => "xor", 7 => "cmp",
                _ => "???",
            };
            instr.set_mnemonic(mnemonic);
            let (rm_str, consumed) = decode_modrm_rm(code, pos, rex_b, rex_w);
            pos += consumed;
            if pos + 4 > code.len() { return None; }
            let imm = i32::from_le_bytes([code[pos], code[pos+1], code[pos+2], code[pos+3]]);
            pos += 4;
            instr.set_operands(&alloc::format!("{}, {:#x}", rm_str, imm));
        }

        // TEST r/m, r (0x85)
        0x85 => {
            instr.set_mnemonic("test");
            if pos < code.len() {
                let (ops, consumed) = decode_modrm_reg(code, pos, rex_r, rex_b, rex_w, false, false);
                pos += consumed;
                instr.set_operands(&ops);
            }
        }

        // XOR/ADD/SUB/CMP/AND/OR r/m, r (0x31/0x01/0x29/0x39/0x21/0x09)
        0x01 | 0x09 | 0x21 | 0x29 | 0x31 | 0x39 => {
            let mnemonic = match opcode {
                0x01 => "add", 0x09 => "or", 0x21 => "and",
                0x29 => "sub", 0x31 => "xor", 0x39 => "cmp",
                _ => "???",
            };
            instr.set_mnemonic(mnemonic);
            if pos < code.len() {
                let (ops, consumed) = decode_modrm_reg(code, pos, rex_r, rex_b, rex_w, false, false);
                pos += consumed;
                instr.set_operands(&ops);
            }
        }

        // XOR/ADD/SUB/CMP/AND/OR r, r/m (0x33/0x03/0x2B/0x3B/0x23/0x0B)
        0x03 | 0x0B | 0x23 | 0x2B | 0x33 | 0x3B => {
            let mnemonic = match opcode {
                0x03 => "add", 0x0B => "or", 0x23 => "and",
                0x2B => "sub", 0x33 => "xor", 0x3B => "cmp",
                _ => "???",
            };
            instr.set_mnemonic(mnemonic);
            if pos < code.len() {
                let (ops, consumed) = decode_modrm_reg(code, pos, rex_r, rex_b, rex_w, true, false);
                pos += consumed;
                instr.set_operands(&ops);
            }
        }

        // CALL/JMP r/m (0xFF /2 or /4)
        0xFF => {
            if pos >= code.len() { return None; }
            let modrm = code[pos];
            let op_ext = (modrm >> 3) & 7;
            match op_ext {
                2 => instr.set_mnemonic("call"),
                4 => instr.set_mnemonic("jmp"),
                6 => instr.set_mnemonic("push"),
                0 => instr.set_mnemonic("inc"),
                1 => instr.set_mnemonic("dec"),
                _ => instr.set_mnemonic("???"),
            }
            let (rm_str, consumed) = decode_modrm_rm(code, pos, rex_b, rex_w);
            pos += consumed;
            instr.set_operands(&rm_str);
        }

        // Catch-all: emit as db
        _ => {
            instr.set_mnemonic("db");
            instr.set_operands(&alloc::format!("{:#04x}", opcode));
        }
    }

    instr.len = pos as u8;
    let copy_len = pos.min(15);
    instr.bytes[..copy_len].copy_from_slice(&code[..copy_len]);

    Some(instr)
}

/// Decode a block of instructions.
pub fn decode_block(code: &[u8], base_rip: u64, max_instrs: usize) -> alloc::vec::Vec<(u64, DecodedInstr)> {
    let mut result = alloc::vec::Vec::new();
    let mut offset = 0;

    while offset < code.len() && result.len() < max_instrs {
        let rip = base_rip + offset as u64;
        match decode(&code[offset..], rip) {
            Some(instr) => {
                let len = instr.len as usize;
                if len == 0 { break; }
                result.push((rip, instr));
                offset += len;
            }
            None => break,
        }
    }

    result
}

// ---- ModR/M decoding helpers ----

/// Decode ModR/M byte and return `(operands_string, bytes_consumed)`.
/// If `reg_first`, output is "reg, r/m"; otherwise "r/m, reg".
fn decode_modrm_reg(code: &[u8], start: usize, rex_r: bool, rex_b: bool, rex_w: bool, reg_first: bool, byte_reg: bool) -> (String, usize) {
    if start >= code.len() {
        return (String::from("???"), 1);
    }
    let modrm = code[start];
    let reg_idx = (((modrm >> 3) & 7) | if rex_r { 8 } else { 0 }) as usize;
    let (rm_str, consumed) = decode_modrm_rm(code, start, rex_b, rex_w);

    let regs = if byte_reg { &REG8 } else if rex_w { &REG64 } else { &REG32 };
    let reg_name = if reg_idx < regs.len() { regs[reg_idx] } else { "???" };

    let ops = if reg_first {
        alloc::format!("{}, {}", reg_name, rm_str)
    } else {
        alloc::format!("{}, {}", rm_str, reg_name)
    };

    (ops, consumed)
}

/// Decode the R/M part of ModR/M, returning `(operand_string, bytes_consumed)`.
fn decode_modrm_rm(code: &[u8], start: usize, rex_b: bool, rex_w: bool) -> (String, usize) {
    if start >= code.len() {
        return (String::from("???"), 1);
    }
    let modrm = code[start];
    let md = modrm >> 6;
    let rm = (modrm & 7) | if rex_b { 8 } else { 0 };
    let mut consumed = 1; // For ModR/M byte

    let regs = if rex_w { &REG64 } else { &REG32 };

    if md == 3 {
        // Register direct
        let name = if (rm as usize) < regs.len() { regs[rm as usize] } else { "???" };
        return (String::from(name), consumed);
    }

    // Memory operand
    let base_reg;
    let mut has_sib = false;
    let mut sib_str = String::new();

    if (rm & 7) == 4 {
        // SIB byte follows
        has_sib = true;
        if start + consumed >= code.len() {
            return (String::from("[???]"), consumed);
        }
        let sib = code[start + consumed];
        consumed += 1;
        let scale = 1u8 << (sib >> 6);
        let index = ((sib >> 3) & 7) | if rex_b { 0 } else { 0 }; // Note: REX.X for SIB index
        let base = (sib & 7) | if rex_b { 8 } else { 0 };

        if md == 0 && (base & 7) == 5 {
            base_reg = String::new(); // No base (disp32)
        } else {
            base_reg = String::from(if (base as usize) < REG64.len() { REG64[base as usize] } else { "???" });
        }

        if index != 4 {
            let idx_name = if (index as usize) < REG64.len() { REG64[index as usize] } else { "???" };
            if scale > 1 {
                sib_str = alloc::format!("{}*{}", idx_name, scale);
            } else {
                sib_str = String::from(idx_name);
            }
        }
    } else if md == 0 && (rm & 7) == 5 {
        // RIP-relative
        if start + consumed + 4 > code.len() {
            return (String::from("[rip+???]"), consumed);
        }
        let disp = i32::from_le_bytes([
            code[start + consumed], code[start + consumed + 1],
            code[start + consumed + 2], code[start + consumed + 3],
        ]);
        consumed += 4;
        return (alloc::format!("[rip{:+#x}]", disp), consumed);
    } else {
        base_reg = String::from(if (rm as usize) < REG64.len() { REG64[rm as usize] } else { "???" });
    }

    // Read displacement
    let disp: i64 = match md {
        0 => {
            if has_sib && base_reg.is_empty() {
                if start + consumed + 4 > code.len() { return (String::from("[???]"), consumed); }
                let d = i32::from_le_bytes([
                    code[start + consumed], code[start + consumed + 1],
                    code[start + consumed + 2], code[start + consumed + 3],
                ]);
                consumed += 4;
                d as i64
            } else {
                0
            }
        }
        1 => {
            if start + consumed >= code.len() { return (String::from("[???]"), consumed); }
            let d = code[start + consumed] as i8;
            consumed += 1;
            d as i64
        }
        2 => {
            if start + consumed + 4 > code.len() { return (String::from("[???]"), consumed); }
            let d = i32::from_le_bytes([
                code[start + consumed], code[start + consumed + 1],
                code[start + consumed + 2], code[start + consumed + 3],
            ]);
            consumed += 4;
            d as i64
        }
        _ => 0,
    };

    // Build operand string
    let mut result = String::from("[");
    if !base_reg.is_empty() {
        result.push_str(&base_reg);
        if !sib_str.is_empty() {
            result.push('+');
            result.push_str(&sib_str);
        }
    } else if !sib_str.is_empty() {
        result.push_str(&sib_str);
    }

    if disp != 0 {
        if !base_reg.is_empty() || !sib_str.is_empty() {
            if disp > 0 {
                result.push_str(&alloc::format!("+{:#x}", disp));
            } else {
                result.push_str(&alloc::format!("-{:#x}", -disp));
            }
        } else {
            result.push_str(&alloc::format!("{:#x}", disp));
        }
    }
    result.push(']');

    (result, consumed)
}
