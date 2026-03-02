//! x86 instruction decoder for all CPU modes (Real, Protected, Long).
//!
//! Decodes raw instruction bytes from guest memory into [`DecodedInst`] structs
//! that the executor can consume. Handles the full x86 primary opcode map (all
//! 256 opcodes) and the most important secondary (0F-prefixed) opcodes needed to
//! run real-world BIOS firmware, bootloaders, and OS kernels.
//!
//! The decoder operates in four phases:
//!
//! 1. **Prefix parsing** -- legacy, segment-override, size-override, and REX prefixes.
//! 2. **Opcode fetch** -- single-byte, two-byte (0F xx), or three-byte (0F 38/3A xx) opcodes.
//! 3. **Size resolution** -- operand and address sizes from mode + prefixes + REX.W.
//! 4. **Operand decoding** -- ModR/M, SIB, displacement, and immediate fields.

use crate::error::{Result, VmError};
use crate::flags::OperandSize;
use crate::instruction::{
    DecodedInst, MemOperand, OpcodeMap, Operand, RegOperand, RepPrefix,
};
use crate::memory::MemoryBus;
use crate::registers::SegReg;

// ---------------------------------------------------------------------------
// CpuMode
// ---------------------------------------------------------------------------

/// The current CPU operating mode, which determines default operand/address
/// sizes and which instructions are available.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CpuMode {
    /// 16-bit real mode (power-on default).
    Real16,
    /// 32-bit protected mode (PE=1, L=0 in CS descriptor).
    Protected32,
    /// 64-bit long mode (PE=1, LME=1, LMA=1, L=1 in CS descriptor).
    Long64,
}

// ---------------------------------------------------------------------------
// Decoder (public API)
// ---------------------------------------------------------------------------

/// Stateless x86 instruction decoder parameterized by [`CpuMode`].
///
/// The decoder borrows guest memory through the [`MemoryBus`] trait and
/// produces a fully decoded [`DecodedInst`] describing the instruction at
/// a given RIP.
pub struct Decoder {
    mode: CpuMode,
}

impl Decoder {
    /// Create a new decoder for the given CPU mode.
    pub fn new(mode: CpuMode) -> Self {
        Decoder { mode }
    }

    /// Change the decoder mode (e.g. after the guest switches to protected mode).
    pub fn set_mode(&mut self, mode: CpuMode) {
        self.mode = mode;
    }

    /// Decode one instruction starting at `rip`.
    ///
    /// Returns a [`DecodedInst`] describing the opcode, operands, prefixes, and
    /// byte length of the instruction. The caller advances RIP by
    /// `inst.length` after execution.
    ///
    /// # Errors
    ///
    /// Returns [`VmError::UndefinedOpcode`] for invalid or unsupported encodings,
    /// and [`VmError::FetchFault`] if reading instruction bytes from memory fails.
    pub fn decode(&self, memory: &dyn MemoryBus, rip: u64) -> Result<DecodedInst> {
        let mut cur = DecodeCursor::new(memory, rip, self.mode);
        cur.decode_instruction()
    }
}

// ---------------------------------------------------------------------------
// DecodeCursor -- internal state machine
// ---------------------------------------------------------------------------

/// Maximum x86 instruction length in bytes (hardware-enforced).
const MAX_INST_LEN: usize = 15;

/// Internal cursor that tracks position within the instruction byte stream
/// and accumulates decoded fields.
struct DecodeCursor<'m> {
    mem: &'m dyn MemoryBus,
    /// RIP at instruction start.
    start_rip: u64,
    /// Current read position (absolute address).
    pos: u64,
    /// CPU mode for this decode.
    mode: CpuMode,
    /// Accumulated instruction being built.
    inst: DecodedInst,
}

impl<'m> DecodeCursor<'m> {
    fn new(mem: &'m dyn MemoryBus, rip: u64, mode: CpuMode) -> Self {
        DecodeCursor {
            mem,
            start_rip: rip,
            pos: rip,
            mode,
            inst: DecodedInst::empty(),
        }
    }

    // -- byte fetching helpers ------------------------------------------

    /// Read the next byte and advance the cursor, enforcing the 15-byte limit.
    fn fetch_u8(&mut self) -> Result<u8> {
        if (self.pos - self.start_rip) as usize >= MAX_INST_LEN {
            return Err(VmError::GeneralProtection(0));
        }
        let b = self.mem.read_u8(self.pos).map_err(|_| VmError::FetchFault(self.pos))?;
        self.pos += 1;
        Ok(b)
    }

    /// Read a little-endian u16 (2 bytes).
    fn fetch_u16(&mut self) -> Result<u16> {
        let lo = self.fetch_u8()? as u16;
        let hi = self.fetch_u8()? as u16;
        Ok(lo | (hi << 8))
    }

    /// Read a little-endian u32 (4 bytes).
    fn fetch_u32(&mut self) -> Result<u32> {
        let lo = self.fetch_u16()? as u32;
        let hi = self.fetch_u16()? as u32;
        Ok(lo | (hi << 16))
    }

    /// Read a little-endian u64 (8 bytes).
    fn fetch_u64(&mut self) -> Result<u64> {
        let lo = self.fetch_u32()? as u64;
        let hi = self.fetch_u32()? as u64;
        Ok(lo | (hi << 32))
    }

    /// Read a sign-extended immediate of the given size.
    fn fetch_imm(&mut self, size: OperandSize) -> Result<u64> {
        match size {
            OperandSize::Byte => {
                let v = self.fetch_u8()? as i8 as i64 as u64;
                Ok(v)
            }
            OperandSize::Word => {
                let v = self.fetch_u16()? as i16 as i64 as u64;
                Ok(v)
            }
            OperandSize::Dword => {
                let v = self.fetch_u32()? as i32 as i64 as u64;
                Ok(v)
            }
            OperandSize::Qword => self.fetch_u64(),
        }
    }

    /// Read an unsigned immediate of the given size (no sign extension).
    fn fetch_uimm(&mut self, size: OperandSize) -> Result<u64> {
        match size {
            OperandSize::Byte => Ok(self.fetch_u8()? as u64),
            OperandSize::Word => Ok(self.fetch_u16()? as u64),
            OperandSize::Dword => Ok(self.fetch_u32()? as u64),
            OperandSize::Qword => self.fetch_u64(),
        }
    }

    /// Bytes consumed so far.
    fn bytes_consumed(&self) -> u8 {
        (self.pos - self.start_rip) as u8
    }

    // -- main decode entry point ----------------------------------------

    /// Run all four decoding phases and return the finished instruction.
    fn decode_instruction(&mut self) -> Result<DecodedInst> {
        self.phase1_prefixes()?;
        self.phase2_opcode()?;
        self.phase3_sizes();
        self.phase4_operands()?;

        self.inst.length = self.bytes_consumed();
        Ok(self.inst.clone())
    }

    // ===================================================================
    // Phase 1: Prefix parsing
    // ===================================================================

    fn phase1_prefixes(&mut self) -> Result<()> {
        loop {
            let b = self.fetch_u8()?;
            match b {
                // -- LOCK / REP / REPNE --
                0xF0 => self.inst.prefix.lock = true,
                0xF2 => self.inst.rep = RepPrefix::Repne,
                0xF3 => self.inst.rep = RepPrefix::Rep,

                // -- Segment overrides --
                0x26 => self.inst.prefix.seg_override = Some(SegReg::Es),
                0x2E => self.inst.prefix.seg_override = Some(SegReg::Cs),
                0x36 => self.inst.prefix.seg_override = Some(SegReg::Ss),
                0x3E => self.inst.prefix.seg_override = Some(SegReg::Ds),
                0x64 => self.inst.prefix.seg_override = Some(SegReg::Fs),
                0x65 => self.inst.prefix.seg_override = Some(SegReg::Gs),

                // -- Operand-size override --
                0x66 => self.inst.prefix.operand_size_override = true,

                // -- Address-size override --
                0x67 => self.inst.prefix.address_size_override = true,

                // -- REX prefix (only valid in Long64 mode) --
                0x40..=0x4F if self.mode == CpuMode::Long64 => {
                    // Store the raw REX byte; bits [3:0] = W R X B.
                    self.inst.prefix.rex = b & 0x0F;
                }

                // Not a prefix -- rewind one byte and exit the loop.
                _ => {
                    self.pos -= 1;
                    break;
                }
            }
        }
        Ok(())
    }

    // ===================================================================
    // Phase 2: Opcode fetch
    // ===================================================================

    fn phase2_opcode(&mut self) -> Result<()> {
        let b1 = self.fetch_u8()?;
        if b1 == 0x0F {
            let b2 = self.fetch_u8()?;
            if b2 == 0x38 {
                let b3 = self.fetch_u8()?;
                self.inst.opcode_map = OpcodeMap::Escape0F38;
                self.inst.opcode = b3 as u16;
            } else if b2 == 0x3A {
                let b3 = self.fetch_u8()?;
                self.inst.opcode_map = OpcodeMap::Escape0F3A;
                self.inst.opcode = b3 as u16;
            } else {
                self.inst.opcode_map = OpcodeMap::Secondary;
                self.inst.opcode = 0x0F00 | (b2 as u16);
            }
        } else {
            self.inst.opcode_map = OpcodeMap::Primary;
            self.inst.opcode = b1 as u16;
        }
        Ok(())
    }

    // ===================================================================
    // Phase 3: Resolve operand and address sizes
    // ===================================================================

    fn phase3_sizes(&mut self) {
        let prefix = &self.inst.prefix;
        match self.mode {
            CpuMode::Long64 => {
                // Default operand = Dword; REX.W -> Qword; 0x66 -> Word.
                self.inst.operand_size = if prefix.rex_w() {
                    OperandSize::Qword
                } else if prefix.operand_size_override {
                    OperandSize::Word
                } else {
                    OperandSize::Dword
                };
                // Default address = Qword; 0x67 -> Dword.
                self.inst.address_size = if prefix.address_size_override {
                    OperandSize::Dword
                } else {
                    OperandSize::Qword
                };
            }
            CpuMode::Protected32 => {
                self.inst.operand_size = if prefix.operand_size_override {
                    OperandSize::Word
                } else {
                    OperandSize::Dword
                };
                self.inst.address_size = if prefix.address_size_override {
                    OperandSize::Word
                } else {
                    OperandSize::Dword
                };
            }
            CpuMode::Real16 => {
                self.inst.operand_size = if prefix.operand_size_override {
                    OperandSize::Dword
                } else {
                    OperandSize::Word
                };
                self.inst.address_size = if prefix.address_size_override {
                    OperandSize::Dword
                } else {
                    OperandSize::Word
                };
            }
        }

        // Some opcodes force 64-bit operand size in Long64 mode regardless
        // of REX.W or 0x66. We apply the override here so phase 4 sees the
        // correct operand_size already.
        if self.mode == CpuMode::Long64 && self.inst.opcode_map == OpcodeMap::Primary {
            if is_default_64(self.inst.opcode) {
                // 0x66 can still narrow these to 16-bit for PUSH/POP imm
                // variants, but the register forms default to 64 in long mode.
                if !self.inst.prefix.operand_size_override {
                    self.inst.operand_size = OperandSize::Qword;
                }
            }
        }
    }

    // ===================================================================
    // Phase 4: Decode operands
    // ===================================================================

    fn phase4_operands(&mut self) -> Result<()> {
        match self.inst.opcode_map {
            OpcodeMap::Primary => self.decode_primary(),
            OpcodeMap::Secondary => self.decode_secondary(),
            OpcodeMap::Escape0F38 | OpcodeMap::Escape0F3A => {
                // Minimal handling: read ModR/M if present and mark as
                // undefined for now -- full SSE/AVX decoding is future work.
                Err(VmError::UndefinedOpcode(self.inst.opcode as u8))
            }
        }
    }

    // ===================================================================
    // Primary opcode map (all 256 opcodes)
    // ===================================================================

    fn decode_primary(&mut self) -> Result<()> {
        let op = self.inst.opcode as u8;
        match op {
            // -- ALU ops: ADD (00-05), OR (08-0D), ADC (10-15), SBB (18-1D),
            //             AND (20-25), SUB (28-2D), XOR (30-35), CMP (38-3D)
            0x00 | 0x08 | 0x10 | 0x18 | 0x20 | 0x28 | 0x30 | 0x38 => {
                // r/m8, r8
                self.decode_modrm_rm_r(OperandSize::Byte)
            }
            0x01 | 0x09 | 0x11 | 0x19 | 0x21 | 0x29 | 0x31 | 0x39 => {
                // r/m, r
                let sz = self.inst.operand_size;
                self.decode_modrm_rm_r(sz)
            }
            0x02 | 0x0A | 0x12 | 0x1A | 0x22 | 0x2A | 0x32 | 0x3A => {
                // r8, r/m8
                self.decode_modrm_r_rm(OperandSize::Byte)
            }
            0x03 | 0x0B | 0x13 | 0x1B | 0x23 | 0x2B | 0x33 | 0x3B => {
                // r, r/m
                let sz = self.inst.operand_size;
                self.decode_modrm_r_rm(sz)
            }
            0x04 | 0x0C | 0x14 | 0x1C | 0x24 | 0x2C | 0x34 | 0x3C => {
                // AL, imm8
                self.decode_al_imm8()
            }
            0x05 | 0x0D | 0x15 | 0x1D | 0x25 | 0x2D | 0x35 | 0x3D => {
                // eAX, imm
                self.decode_eax_imm()
            }

            // -- PUSH/POP segment, BCD adjusts (invalid in 64-bit) --
            0x06 | 0x07 | 0x0E | 0x16 | 0x17 | 0x1E | 0x1F => {
                self.require_not_64()?;
                // PUSH/POP ES/CS/SS/DS — no further operand bytes.
                let seg = match op {
                    0x06 | 0x07 => SegReg::Es,
                    0x0E => SegReg::Cs,
                    0x16 | 0x17 => SegReg::Ss,
                    0x1E | 0x1F => SegReg::Ds,
                    _ => unreachable!(),
                };
                self.set_operand(0, Operand::Register(RegOperand::Seg(seg)));
                self.inst.operand_count = 1;
                Ok(())
            }
            0x27 | 0x2F | 0x37 | 0x3F => {
                // DAA / DAS / AAA / AAS
                self.require_not_64()?;
                self.inst.operand_count = 0;
                Ok(())
            }

            // -- INC/DEC r16/r32 (invalid in 64-bit, where these are REX) --
            0x40..=0x4F => {
                self.require_not_64()?;
                let reg = op & 0x07;
                let sz = self.inst.operand_size;
                self.set_operand(0, self.gpr_operand(reg, sz));
                self.inst.operand_count = 1;
                Ok(())
            }

            // -- PUSH r --
            0x50..=0x57 => self.decode_push_pop_reg(op),
            // -- POP r --
            0x58..=0x5F => self.decode_push_pop_reg(op),

            // -- PUSHA / POPA (invalid in 64-bit) --
            0x60 | 0x61 => {
                self.require_not_64()?;
                self.inst.operand_count = 0;
                Ok(())
            }

            // -- BOUND (invalid in 64-bit) --
            0x62 => {
                self.require_not_64()?;
                let sz = self.inst.operand_size;
                self.decode_modrm_r_rm(sz)
            }

            // -- ARPL (32-bit) / MOVSXD (64-bit) --
            0x63 => {
                if self.mode == CpuMode::Long64 {
                    // MOVSXD r, r/m32 (REX.W -> r64, r/m32)
                    let sz = self.inst.operand_size;
                    self.decode_modrm_r_rm(sz)
                } else {
                    // ARPL r/m16, r16
                    self.decode_modrm_rm_r(OperandSize::Word)
                }
            }

            // -- Escape / prefix bytes (consumed in earlier phases) --
            // 0x0F: two-byte escape (consumed in phase 2).
            // Segment overrides (0x26=ES, 0x2E=CS, 0x36=SS, 0x3E=DS, 0x64=FS,
            // 0x65=GS), size overrides (0x66, 0x67) consumed in phase 1.
            // These should never reach the primary opcode match in practice.
            0x0F | 0x26 | 0x2E | 0x36 | 0x3E | 0x64 | 0x65 | 0x66 | 0x67 => {
                self.inst.operand_count = 0;
                Ok(())
            }

            // -- PUSH imm16/imm32 --
            0x68 => {
                let sz = self.inst.operand_size;
                let imm_sz = if sz == OperandSize::Qword { OperandSize::Dword } else { sz };
                let imm = self.fetch_imm(imm_sz)?;
                self.inst.immediate = imm;
                self.set_operand(0, Operand::Immediate(imm));
                self.inst.operand_count = 1;
                Ok(())
            }

            // -- IMUL r, r/m, imm --
            0x69 => {
                let sz = self.inst.operand_size;
                self.decode_modrm_r_rm(sz)?;
                let imm_sz = if sz == OperandSize::Qword { OperandSize::Dword } else { sz };
                let imm = self.fetch_imm(imm_sz)?;
                self.inst.immediate = imm;
                self.set_operand(2, Operand::Immediate(imm));
                self.inst.operand_count = 3;
                Ok(())
            }

            // -- PUSH imm8 (sign-extended) --
            0x6A => {
                let imm = self.fetch_imm(OperandSize::Byte)?;
                self.inst.immediate = imm;
                self.set_operand(0, Operand::Immediate(imm));
                self.inst.operand_count = 1;
                Ok(())
            }

            // -- IMUL r, r/m, imm8 --
            0x6B => {
                let sz = self.inst.operand_size;
                self.decode_modrm_r_rm(sz)?;
                let imm = self.fetch_imm(OperandSize::Byte)?;
                self.inst.immediate = imm;
                self.set_operand(2, Operand::Immediate(imm));
                self.inst.operand_count = 3;
                Ok(())
            }

            // -- INS/OUTS (string I/O) --
            0x6C => {
                // INSB
                self.inst.operand_count = 0;
                Ok(())
            }
            0x6D => {
                // INSW/INSD
                self.inst.operand_count = 0;
                Ok(())
            }
            0x6E => {
                // OUTSB
                self.inst.operand_count = 0;
                Ok(())
            }
            0x6F => {
                // OUTSW/OUTSD
                self.inst.operand_count = 0;
                Ok(())
            }

            // -- Jcc short (rel8) --
            0x70..=0x7F => {
                let rel = self.fetch_u8()? as i8 as i64;
                self.set_operand(0, Operand::RelativeOffset(rel));
                self.inst.operand_count = 1;
                Ok(())
            }

            // -- Group 1 (ADD/OR/ADC/SBB/AND/SUB/XOR/CMP r/m, imm) --
            0x80 => self.decode_group_rm_imm(OperandSize::Byte, OperandSize::Byte),
            0x81 => {
                let sz = self.inst.operand_size;
                // Immediate is Dword even in Qword mode (sign-extended by executor).
                let imm_sz = if sz == OperandSize::Qword { OperandSize::Dword } else { sz };
                self.decode_group_rm_imm(sz, imm_sz)
            }
            0x82 => {
                // Alias of 0x80 in 32-bit mode; #UD in 64-bit.
                if self.mode == CpuMode::Long64 {
                    return Err(VmError::UndefinedOpcode(op));
                }
                self.decode_group_rm_imm(OperandSize::Byte, OperandSize::Byte)
            }
            0x83 => {
                let sz = self.inst.operand_size;
                self.decode_group_rm_imm(sz, OperandSize::Byte)
            }

            // -- TEST r/m, r --
            0x84 => self.decode_modrm_rm_r(OperandSize::Byte),
            0x85 => {
                let sz = self.inst.operand_size;
                self.decode_modrm_rm_r(sz)
            }

            // -- XCHG r/m, r --
            0x86 => self.decode_modrm_rm_r(OperandSize::Byte),
            0x87 => {
                let sz = self.inst.operand_size;
                self.decode_modrm_rm_r(sz)
            }

            // -- MOV r/m8,r8 | r/m,r | r8,r/m8 | r,r/m --
            0x88 => self.decode_modrm_rm_r(OperandSize::Byte),
            0x89 => {
                let sz = self.inst.operand_size;
                self.decode_modrm_rm_r(sz)
            }
            0x8A => self.decode_modrm_r_rm(OperandSize::Byte),
            0x8B => {
                let sz = self.inst.operand_size;
                self.decode_modrm_r_rm(sz)
            }

            // -- MOV r/m16, Sreg --
            0x8C => {
                let modrm = self.fetch_modrm()?;
                let (md, reg, rm) = Self::split_modrm(modrm);
                let seg = SegReg::from_encoding(reg)
                    .ok_or(VmError::UndefinedOpcode(op))?;
                let rm_op = self.decode_rm(md, rm, OperandSize::Word)?;
                self.set_operand(0, rm_op);
                self.set_operand(1, Operand::Register(RegOperand::Seg(seg)));
                self.inst.operand_count = 2;
                Ok(())
            }

            // -- LEA r, m --
            0x8D => {
                let sz = self.inst.operand_size;
                self.decode_modrm_r_rm(sz)
            }

            // -- MOV Sreg, r/m16 --
            0x8E => {
                let modrm = self.fetch_modrm()?;
                let (md, reg, rm) = Self::split_modrm(modrm);
                let seg = SegReg::from_encoding(reg)
                    .ok_or(VmError::UndefinedOpcode(op))?;
                let rm_op = self.decode_rm(md, rm, OperandSize::Word)?;
                self.set_operand(0, Operand::Register(RegOperand::Seg(seg)));
                self.set_operand(1, rm_op);
                self.inst.operand_count = 2;
                Ok(())
            }

            // -- POP r/m --
            0x8F => {
                let sz = self.inst.operand_size;
                let modrm = self.fetch_modrm()?;
                let (md, _reg, rm) = Self::split_modrm(modrm);
                let rm_op = self.decode_rm(md, rm, sz)?;
                self.set_operand(0, rm_op);
                self.inst.operand_count = 1;
                Ok(())
            }

            // -- NOP / XCHG eAX, r --
            0x90..=0x97 => {
                let reg_bits = op & 0x07;
                let reg = self.extend_b(reg_bits);
                // 0x90 without REX.B is NOP; with REX.B it is XCHG RAX, R8.
                if op == 0x90 && !self.inst.prefix.rex_b() {
                    self.inst.operand_count = 0;
                    return Ok(());
                }
                let sz = self.inst.operand_size;
                self.set_operand(0, self.gpr_operand(0, sz)); // eAX
                self.set_operand(1, self.gpr_operand(reg, sz));
                self.inst.operand_count = 2;
                Ok(())
            }

            // -- CBW/CWDE/CDQE, CWD/CDQ/CQO --
            0x98 | 0x99 => {
                self.inst.operand_count = 0;
                Ok(())
            }

            // -- CALL far (ptr16:16/32) -- invalid in 64-bit --
            0x9A => {
                self.require_not_64()?;
                let sz = self.inst.operand_size;
                let offset = self.fetch_uimm(sz)?;
                let seg = self.fetch_u16()?;
                self.set_operand(0, Operand::FarPointer { segment: seg, offset });
                self.inst.operand_count = 1;
                Ok(())
            }

            // -- FWAIT --
            0x9B => {
                self.inst.operand_count = 0;
                Ok(())
            }

            // -- PUSHF / POPF --
            0x9C | 0x9D => {
                self.inst.operand_count = 0;
                Ok(())
            }

            // -- SAHF / LAHF --
            0x9E | 0x9F => {
                self.inst.operand_count = 0;
                Ok(())
            }

            // -- MOV AL/AX/EAX/RAX, moffs | MOV moffs, AL/AX/EAX/RAX --
            0xA0 => {
                let addr = self.fetch_moffs()?;
                self.set_operand(0, self.gpr_operand(0, OperandSize::Byte)); // AL
                self.set_operand(1, self.moffs_operand(addr, OperandSize::Byte));
                self.inst.operand_count = 2;
                Ok(())
            }
            0xA1 => {
                let sz = self.inst.operand_size;
                let addr = self.fetch_moffs()?;
                self.set_operand(0, self.gpr_operand(0, sz)); // eAX/rAX
                self.set_operand(1, self.moffs_operand(addr, sz));
                self.inst.operand_count = 2;
                Ok(())
            }
            0xA2 => {
                let addr = self.fetch_moffs()?;
                self.set_operand(0, self.moffs_operand(addr, OperandSize::Byte));
                self.set_operand(1, self.gpr_operand(0, OperandSize::Byte)); // AL
                self.inst.operand_count = 2;
                Ok(())
            }
            0xA3 => {
                let sz = self.inst.operand_size;
                let addr = self.fetch_moffs()?;
                self.set_operand(0, self.moffs_operand(addr, sz));
                self.set_operand(1, self.gpr_operand(0, sz)); // eAX/rAX
                self.inst.operand_count = 2;
                Ok(())
            }

            // -- String ops --
            0xA4 | 0xA5 | 0xA6 | 0xA7 | 0xAA | 0xAB | 0xAC | 0xAD | 0xAE | 0xAF => {
                // MOVS, CMPS, STOS, LODS, SCAS -- implicit operands only.
                self.inst.operand_count = 0;
                Ok(())
            }

            // -- TEST AL, imm8 --
            0xA8 => self.decode_al_imm8(),
            // -- TEST eAX, imm --
            0xA9 => self.decode_eax_imm(),

            // -- MOV r8, imm8 --
            0xB0..=0xB7 => {
                let reg = self.extend_b(op & 0x07);
                let imm = self.fetch_u8()? as u64;
                self.inst.immediate = imm;
                self.set_operand(0, self.gpr_operand(reg, OperandSize::Byte));
                self.set_operand(1, Operand::Immediate(imm));
                self.inst.operand_count = 2;
                Ok(())
            }

            // -- MOV r, imm (32 or 64-bit immediate with REX.W) --
            0xB8..=0xBF => {
                let reg = self.extend_b(op & 0x07);
                let sz = self.inst.operand_size;
                // In 64-bit mode with REX.W, the immediate is a full 64 bits.
                let imm = self.fetch_uimm(sz)?;
                self.inst.immediate = imm;
                self.set_operand(0, self.gpr_operand(reg, sz));
                self.set_operand(1, Operand::Immediate(imm));
                self.inst.operand_count = 2;
                Ok(())
            }

            // -- Group 2 (shifts): r/m8, imm8 / r/m, imm8 --
            0xC0 => self.decode_shift_imm8(OperandSize::Byte),
            0xC1 => {
                let sz = self.inst.operand_size;
                self.decode_shift_imm8(sz)
            }

            // -- RET near imm16 --
            0xC2 => {
                let imm = self.fetch_u16()? as u64;
                self.inst.immediate = imm;
                self.set_operand(0, Operand::Immediate(imm));
                self.inst.operand_count = 1;
                Ok(())
            }

            // -- RET near --
            0xC3 => {
                self.inst.operand_count = 0;
                Ok(())
            }

            // -- LES/LDS (invalid in 64-bit) --
            0xC4 | 0xC5 => {
                self.require_not_64()?;
                let sz = self.inst.operand_size;
                self.decode_modrm_r_rm(sz)
            }

            // -- MOV r/m8, imm8 --
            0xC6 => {
                let modrm = self.fetch_modrm()?;
                let (md, _reg, rm) = Self::split_modrm(modrm);
                let rm_op = self.decode_rm(md, rm, OperandSize::Byte)?;
                let imm = self.fetch_u8()? as u64;
                self.inst.immediate = imm;
                self.set_operand(0, rm_op);
                self.set_operand(1, Operand::Immediate(imm));
                self.inst.operand_count = 2;
                Ok(())
            }

            // -- MOV r/m, imm --
            0xC7 => {
                let sz = self.inst.operand_size;
                let modrm = self.fetch_modrm()?;
                let (md, _reg, rm) = Self::split_modrm(modrm);
                let rm_op = self.decode_rm(md, rm, sz)?;
                // In 64-bit mode, immediate is still 32-bit (sign-extended by executor).
                let imm_sz = if sz == OperandSize::Qword { OperandSize::Dword } else { sz };
                let imm = self.fetch_imm(imm_sz)?;
                self.inst.immediate = imm;
                self.set_operand(0, rm_op);
                self.set_operand(1, Operand::Immediate(imm));
                self.inst.operand_count = 2;
                Ok(())
            }

            // -- ENTER imm16, imm8 --
            0xC8 => {
                let imm16 = self.fetch_u16()? as u64;
                let imm8 = self.fetch_u8()? as u64;
                self.inst.immediate = imm16;
                self.inst.immediate2 = imm8;
                self.set_operand(0, Operand::Immediate(imm16));
                self.set_operand(1, Operand::Immediate(imm8));
                self.inst.operand_count = 2;
                Ok(())
            }

            // -- LEAVE --
            0xC9 => {
                self.inst.operand_count = 0;
                Ok(())
            }

            // -- RETF imm16 --
            0xCA => {
                let imm = self.fetch_u16()? as u64;
                self.inst.immediate = imm;
                self.set_operand(0, Operand::Immediate(imm));
                self.inst.operand_count = 1;
                Ok(())
            }

            // -- RETF --
            0xCB => {
                self.inst.operand_count = 0;
                Ok(())
            }

            // -- INT 3 --
            0xCC => {
                self.inst.operand_count = 0;
                Ok(())
            }

            // -- INT imm8 --
            0xCD => {
                let imm = self.fetch_u8()? as u64;
                self.inst.immediate = imm;
                self.set_operand(0, Operand::Immediate(imm));
                self.inst.operand_count = 1;
                Ok(())
            }

            // -- INTO (invalid in 64-bit) --
            0xCE => {
                self.require_not_64()?;
                self.inst.operand_count = 0;
                Ok(())
            }

            // -- IRET/IRETD/IRETQ --
            0xCF => {
                self.inst.operand_count = 0;
                Ok(())
            }

            // -- Group 2 (shifts): r/m, 1 --
            0xD0 => self.decode_shift_one(OperandSize::Byte),
            0xD1 => {
                let sz = self.inst.operand_size;
                self.decode_shift_one(sz)
            }

            // -- Group 2 (shifts): r/m, CL --
            0xD2 => self.decode_shift_cl(OperandSize::Byte),
            0xD3 => {
                let sz = self.inst.operand_size;
                self.decode_shift_cl(sz)
            }

            // -- AAM / AAD (invalid in 64-bit) --
            0xD4 | 0xD5 => {
                self.require_not_64()?;
                let imm = self.fetch_u8()? as u64;
                self.inst.immediate = imm;
                self.set_operand(0, Operand::Immediate(imm));
                self.inst.operand_count = 1;
                Ok(())
            }

            // -- 0xD6: SALC (undocumented) / #UD --
            0xD6 => Err(VmError::UndefinedOpcode(op)),

            // -- XLAT --
            0xD7 => {
                self.inst.operand_count = 0;
                Ok(())
            }

            // -- x87 FPU escape (D8-DF) --
            0xD8..=0xDF => {
                // Read ModR/M so the instruction length is correct; the
                // executor or a dedicated FPU handler interprets the opcode.
                let modrm = self.fetch_modrm()?;
                let (md, _reg, rm) = Self::split_modrm(modrm);
                if md != 3 {
                    // Memory operand -- consume displacement / SIB.
                    let _ = self.decode_rm(md, rm, OperandSize::Dword)?;
                }
                self.inst.operand_count = 0;
                Ok(())
            }

            // -- LOOP/LOOPE/LOOPNE/JCXZ (rel8) --
            0xE0..=0xE3 => {
                let rel = self.fetch_u8()? as i8 as i64;
                self.set_operand(0, Operand::RelativeOffset(rel));
                self.inst.operand_count = 1;
                Ok(())
            }

            // -- IN AL, imm8 --
            0xE4 => {
                let imm = self.fetch_u8()? as u64;
                self.inst.immediate = imm;
                self.set_operand(0, self.gpr_operand(0, OperandSize::Byte)); // AL
                self.set_operand(1, Operand::Immediate(imm));
                self.inst.operand_count = 2;
                Ok(())
            }
            // -- IN eAX, imm8 --
            0xE5 => {
                let sz = self.inst.operand_size;
                let imm = self.fetch_u8()? as u64;
                self.inst.immediate = imm;
                self.set_operand(0, self.gpr_operand(0, sz));
                self.set_operand(1, Operand::Immediate(imm));
                self.inst.operand_count = 2;
                Ok(())
            }
            // -- OUT imm8, AL --
            0xE6 => {
                let imm = self.fetch_u8()? as u64;
                self.inst.immediate = imm;
                self.set_operand(0, Operand::Immediate(imm));
                self.set_operand(1, self.gpr_operand(0, OperandSize::Byte)); // AL
                self.inst.operand_count = 2;
                Ok(())
            }
            // -- OUT imm8, eAX --
            0xE7 => {
                let sz = self.inst.operand_size;
                let imm = self.fetch_u8()? as u64;
                self.inst.immediate = imm;
                self.set_operand(0, Operand::Immediate(imm));
                self.set_operand(1, self.gpr_operand(0, sz));
                self.inst.operand_count = 2;
                Ok(())
            }

            // -- CALL near rel16/rel32 --
            0xE8 => self.decode_rel_operand(),

            // -- JMP near rel16/rel32 --
            0xE9 => self.decode_rel_operand(),

            // -- JMP far (ptr16:16/32) -- invalid in 64-bit --
            0xEA => {
                self.require_not_64()?;
                let sz = self.inst.operand_size;
                let offset = self.fetch_uimm(sz)?;
                let seg = self.fetch_u16()?;
                self.set_operand(0, Operand::FarPointer { segment: seg, offset });
                self.inst.operand_count = 1;
                Ok(())
            }

            // -- JMP short rel8 --
            0xEB => {
                let rel = self.fetch_u8()? as i8 as i64;
                self.set_operand(0, Operand::RelativeOffset(rel));
                self.inst.operand_count = 1;
                Ok(())
            }

            // -- IN AL, DX --
            0xEC => {
                self.set_operand(0, self.gpr_operand(0, OperandSize::Byte)); // AL
                self.set_operand(1, self.gpr_operand(2, OperandSize::Word)); // DX
                self.inst.operand_count = 2;
                Ok(())
            }
            // -- IN eAX, DX --
            0xED => {
                let sz = self.inst.operand_size;
                self.set_operand(0, self.gpr_operand(0, sz));
                self.set_operand(1, self.gpr_operand(2, OperandSize::Word)); // DX
                self.inst.operand_count = 2;
                Ok(())
            }
            // -- OUT DX, AL --
            0xEE => {
                self.set_operand(0, self.gpr_operand(2, OperandSize::Word)); // DX
                self.set_operand(1, self.gpr_operand(0, OperandSize::Byte)); // AL
                self.inst.operand_count = 2;
                Ok(())
            }
            // -- OUT DX, eAX --
            0xEF => {
                let sz = self.inst.operand_size;
                self.set_operand(0, self.gpr_operand(2, OperandSize::Word)); // DX
                self.set_operand(1, self.gpr_operand(0, sz));
                self.inst.operand_count = 2;
                Ok(())
            }

            // -- LOCK prefix (already consumed in phase1) / INT1 --
            0xF0 => {
                // Should not reach here (handled as prefix), but be safe.
                self.inst.operand_count = 0;
                Ok(())
            }
            0xF1 => {
                // INT1 / ICEBP
                self.inst.operand_count = 0;
                Ok(())
            }

            // -- REP/REPNE (already consumed in phase1) --
            0xF2 | 0xF3 => {
                self.inst.operand_count = 0;
                Ok(())
            }

            // -- HLT --
            0xF4 => {
                self.inst.operand_count = 0;
                Ok(())
            }

            // -- CMC --
            0xF5 => {
                self.inst.operand_count = 0;
                Ok(())
            }

            // -- Group 3: Unary (TEST/NOT/NEG/MUL/IMUL/DIV/IDIV) r/m8 --
            0xF6 => self.decode_group3(OperandSize::Byte),

            // -- Group 3: Unary r/m --
            0xF7 => {
                let sz = self.inst.operand_size;
                self.decode_group3(sz)
            }

            // -- CLC / STC / CLI / STI / CLD / STD --
            0xF8 | 0xF9 | 0xFA | 0xFB | 0xFC | 0xFD => {
                self.inst.operand_count = 0;
                Ok(())
            }

            // -- Group 4: INC/DEC r/m8 --
            0xFE => {
                let modrm = self.fetch_modrm()?;
                let (md, _reg, rm) = Self::split_modrm(modrm);
                let rm_op = self.decode_rm(md, rm, OperandSize::Byte)?;
                self.set_operand(0, rm_op);
                self.inst.operand_count = 1;
                Ok(())
            }

            // -- Group 5: INC/DEC/CALL/JMP/PUSH r/m --
            0xFF => {
                let sz = self.inst.operand_size;
                let modrm = self.fetch_modrm()?;
                let (md, reg, rm) = Self::split_modrm(modrm);
                // For CALL/JMP indirect in 64-bit mode, operand size is
                // forced to 64-bit.
                let eff_sz = if self.mode == CpuMode::Long64 && (reg == 2 || reg == 4) {
                    OperandSize::Qword
                } else {
                    sz
                };
                let rm_op = self.decode_rm(md, rm, eff_sz)?;
                self.set_operand(0, rm_op);
                self.inst.operand_count = 1;
                Ok(())
            }
        }
    }

    // ===================================================================
    // Secondary opcode map (0F xx)
    // ===================================================================

    fn decode_secondary(&mut self) -> Result<()> {
        let op = self.inst.opcode;
        let op_lo = (op & 0xFF) as u8;

        match op_lo {
            // -- Group 6: SLDT/STR/LLDT/LTR/VERR/VERW --
            0x00 => {
                let modrm = self.fetch_modrm()?;
                let (md, _reg, rm) = Self::split_modrm(modrm);
                if md != 3 {
                    let rm_op = self.decode_rm(md, rm, OperandSize::Word)?;
                    self.set_operand(0, rm_op);
                } else {
                    self.set_operand(0, Operand::Register(RegOperand::Gpr(rm)));
                }
                self.inst.operand_count = 1;
                Ok(())
            }

            // -- Group 7: LGDT/LIDT/SMSW/LMSW/INVLPG/etc. --
            0x01 => {
                let modrm = self.fetch_modrm()?;
                let (md, _reg, rm) = Self::split_modrm(modrm);
                if md != 3 {
                    let rm_op = self.decode_rm(md, rm, self.inst.operand_size)?;
                    self.set_operand(0, rm_op);
                    self.inst.operand_count = 1;
                } else {
                    // Register form -- various privileged instructions
                    // (XGETBV, etc.) with no further operand bytes.
                    self.inst.operand_count = 0;
                }
                Ok(())
            }

            // -- SYSCALL --
            0x05 => {
                self.inst.operand_count = 0;
                Ok(())
            }

            // -- SYSRET --
            0x07 => {
                self.inst.operand_count = 0;
                Ok(())
            }

            // -- UD2 --
            0x0B => {
                self.inst.operand_count = 0;
                Ok(())
            }

            // -- MOV r, CRn --
            0x20 => {
                let modrm = self.fetch_modrm()?;
                let (_md, reg, rm) = Self::split_modrm(modrm);
                let cr_reg = self.extend_r(reg);
                let gp_reg = self.extend_b(rm);
                self.set_operand(0, self.gpr_operand(gp_reg, OperandSize::Qword));
                self.set_operand(1, Operand::Register(RegOperand::Cr(cr_reg)));
                self.inst.operand_count = 2;
                Ok(())
            }

            // -- MOV r, DRn --
            0x21 => {
                let modrm = self.fetch_modrm()?;
                let (_md, reg, rm) = Self::split_modrm(modrm);
                let dr_reg = self.extend_r(reg);
                let gp_reg = self.extend_b(rm);
                self.set_operand(0, self.gpr_operand(gp_reg, OperandSize::Qword));
                self.set_operand(1, Operand::Register(RegOperand::Dr(dr_reg)));
                self.inst.operand_count = 2;
                Ok(())
            }

            // -- MOV CRn, r --
            0x22 => {
                let modrm = self.fetch_modrm()?;
                let (_md, reg, rm) = Self::split_modrm(modrm);
                let cr_reg = self.extend_r(reg);
                let gp_reg = self.extend_b(rm);
                self.set_operand(0, Operand::Register(RegOperand::Cr(cr_reg)));
                self.set_operand(1, self.gpr_operand(gp_reg, OperandSize::Qword));
                self.inst.operand_count = 2;
                Ok(())
            }

            // -- MOV DRn, r --
            0x23 => {
                let modrm = self.fetch_modrm()?;
                let (_md, reg, rm) = Self::split_modrm(modrm);
                let dr_reg = self.extend_r(reg);
                let gp_reg = self.extend_b(rm);
                self.set_operand(0, Operand::Register(RegOperand::Dr(dr_reg)));
                self.set_operand(1, self.gpr_operand(gp_reg, OperandSize::Qword));
                self.inst.operand_count = 2;
                Ok(())
            }

            // -- WRMSR --
            0x30 => {
                self.inst.operand_count = 0;
                Ok(())
            }

            // -- RDTSC --
            0x31 => {
                self.inst.operand_count = 0;
                Ok(())
            }

            // -- RDMSR --
            0x32 => {
                self.inst.operand_count = 0;
                Ok(())
            }

            // -- CMOVcc r, r/m --
            0x40..=0x4F => {
                let sz = self.inst.operand_size;
                self.decode_modrm_r_rm(sz)
            }

            // -- Jcc near (rel16/rel32) --
            0x80..=0x8F => self.decode_rel_operand(),

            // -- SETcc r/m8 --
            0x90..=0x9F => {
                let modrm = self.fetch_modrm()?;
                let (md, _reg, rm) = Self::split_modrm(modrm);
                let rm_op = self.decode_rm(md, rm, OperandSize::Byte)?;
                self.set_operand(0, rm_op);
                self.inst.operand_count = 1;
                Ok(())
            }

            // -- PUSH FS --
            0xA0 => {
                self.set_operand(0, Operand::Register(RegOperand::Seg(SegReg::Fs)));
                self.inst.operand_count = 1;
                Ok(())
            }

            // -- POP FS --
            0xA1 => {
                self.set_operand(0, Operand::Register(RegOperand::Seg(SegReg::Fs)));
                self.inst.operand_count = 1;
                Ok(())
            }

            // -- CPUID --
            0xA2 => {
                self.inst.operand_count = 0;
                Ok(())
            }

            // -- BT r/m, r --
            0xA3 => {
                let sz = self.inst.operand_size;
                self.decode_modrm_rm_r(sz)
            }

            // -- SHLD r/m, r, imm8 --
            0xA4 => {
                let sz = self.inst.operand_size;
                self.decode_modrm_rm_r(sz)?;
                let imm = self.fetch_u8()? as u64;
                self.inst.immediate = imm;
                self.set_operand(2, Operand::Immediate(imm));
                self.inst.operand_count = 3;
                Ok(())
            }

            // -- SHLD r/m, r, CL --
            0xA5 => {
                let sz = self.inst.operand_size;
                self.decode_modrm_rm_r(sz)?;
                self.set_operand(2, self.gpr_operand(1, OperandSize::Byte)); // CL
                self.inst.operand_count = 3;
                Ok(())
            }

            // -- PUSH GS --
            0xA8 => {
                self.set_operand(0, Operand::Register(RegOperand::Seg(SegReg::Gs)));
                self.inst.operand_count = 1;
                Ok(())
            }

            // -- POP GS --
            0xA9 => {
                self.set_operand(0, Operand::Register(RegOperand::Seg(SegReg::Gs)));
                self.inst.operand_count = 1;
                Ok(())
            }

            // -- BTS r/m, r --
            0xAB => {
                let sz = self.inst.operand_size;
                self.decode_modrm_rm_r(sz)
            }

            // -- SHRD r/m, r, imm8 --
            0xAC => {
                let sz = self.inst.operand_size;
                self.decode_modrm_rm_r(sz)?;
                let imm = self.fetch_u8()? as u64;
                self.inst.immediate = imm;
                self.set_operand(2, Operand::Immediate(imm));
                self.inst.operand_count = 3;
                Ok(())
            }

            // -- SHRD r/m, r, CL --
            0xAD => {
                let sz = self.inst.operand_size;
                self.decode_modrm_rm_r(sz)?;
                self.set_operand(2, self.gpr_operand(1, OperandSize::Byte)); // CL
                self.inst.operand_count = 3;
                Ok(())
            }

            // -- IMUL r, r/m --
            0xAF => {
                let sz = self.inst.operand_size;
                self.decode_modrm_r_rm(sz)
            }

            // -- CMPXCHG r/m8, r8 --
            0xB0 => self.decode_modrm_rm_r(OperandSize::Byte),

            // -- CMPXCHG r/m, r --
            0xB1 => {
                let sz = self.inst.operand_size;
                self.decode_modrm_rm_r(sz)
            }

            // -- BTR r/m, r --
            0xB3 => {
                let sz = self.inst.operand_size;
                self.decode_modrm_rm_r(sz)
            }

            // -- MOVZX r, r/m8 --
            0xB6 => {
                let sz = self.inst.operand_size;
                let modrm = self.fetch_modrm()?;
                let (md, reg, rm) = Self::split_modrm(modrm);
                let reg = self.extend_r(reg);
                let rm_op = self.decode_rm(md, rm, OperandSize::Byte)?;
                self.set_operand(0, self.gpr_operand(reg, sz));
                self.set_operand(1, rm_op);
                self.inst.operand_count = 2;
                Ok(())
            }

            // -- MOVZX r, r/m16 --
            0xB7 => {
                let sz = self.inst.operand_size;
                let modrm = self.fetch_modrm()?;
                let (md, reg, rm) = Self::split_modrm(modrm);
                let reg = self.extend_r(reg);
                let rm_op = self.decode_rm(md, rm, OperandSize::Word)?;
                self.set_operand(0, self.gpr_operand(reg, sz));
                self.set_operand(1, rm_op);
                self.inst.operand_count = 2;
                Ok(())
            }

            // -- Group 8: BT/BTS/BTR/BTC r/m, imm8 --
            0xBA => {
                let sz = self.inst.operand_size;
                let modrm = self.fetch_modrm()?;
                let (md, _reg, rm) = Self::split_modrm(modrm);
                let rm_op = self.decode_rm(md, rm, sz)?;
                let imm = self.fetch_u8()? as u64;
                self.inst.immediate = imm;
                self.set_operand(0, rm_op);
                self.set_operand(1, Operand::Immediate(imm));
                self.inst.operand_count = 2;
                Ok(())
            }

            // -- BTC r/m, r --
            0xBB => {
                let sz = self.inst.operand_size;
                self.decode_modrm_rm_r(sz)
            }

            // -- BSF r, r/m --
            0xBC => {
                let sz = self.inst.operand_size;
                self.decode_modrm_r_rm(sz)
            }

            // -- BSR r, r/m --
            0xBD => {
                let sz = self.inst.operand_size;
                self.decode_modrm_r_rm(sz)
            }

            // -- MOVSX r, r/m8 --
            0xBE => {
                let sz = self.inst.operand_size;
                let modrm = self.fetch_modrm()?;
                let (md, reg, rm) = Self::split_modrm(modrm);
                let reg = self.extend_r(reg);
                let rm_op = self.decode_rm(md, rm, OperandSize::Byte)?;
                self.set_operand(0, self.gpr_operand(reg, sz));
                self.set_operand(1, rm_op);
                self.inst.operand_count = 2;
                Ok(())
            }

            // -- MOVSX r, r/m16 --
            0xBF => {
                let sz = self.inst.operand_size;
                let modrm = self.fetch_modrm()?;
                let (md, reg, rm) = Self::split_modrm(modrm);
                let reg = self.extend_r(reg);
                let rm_op = self.decode_rm(md, rm, OperandSize::Word)?;
                self.set_operand(0, self.gpr_operand(reg, sz));
                self.set_operand(1, rm_op);
                self.inst.operand_count = 2;
                Ok(())
            }

            // -- XADD r/m8, r8 --
            0xC0 => self.decode_modrm_rm_r(OperandSize::Byte),

            // -- XADD r/m, r --
            0xC1 => {
                let sz = self.inst.operand_size;
                self.decode_modrm_rm_r(sz)
            }

            // -- BSWAP r32/r64 --
            0xC8..=0xCF => {
                let reg = self.extend_b(op_lo & 0x07);
                let sz = self.inst.operand_size;
                self.set_operand(0, self.gpr_operand(reg, sz));
                self.inst.operand_count = 1;
                Ok(())
            }

            _ => Err(VmError::UndefinedOpcode(op_lo)),
        }
    }

    // ===================================================================
    // ModR/M + SIB decoding
    // ===================================================================

    /// Fetch the ModR/M byte and record it in the instruction.
    fn fetch_modrm(&mut self) -> Result<u8> {
        let b = self.fetch_u8()?;
        self.inst.modrm = Some(b);
        Ok(b)
    }

    /// Split ModR/M into (mod, reg, rm) fields (all 3 bits each).
    #[inline]
    fn split_modrm(modrm: u8) -> (u8, u8, u8) {
        ((modrm >> 6) & 3, (modrm >> 3) & 7, modrm & 7)
    }

    /// Apply REX.B extension to a 3-bit register field.
    #[inline]
    fn extend_b(&self, reg3: u8) -> u8 {
        if self.inst.prefix.rex_b() { reg3 | 8 } else { reg3 }
    }

    /// Apply REX.R extension to a 3-bit register field.
    #[inline]
    fn extend_r(&self, reg3: u8) -> u8 {
        if self.inst.prefix.rex_r() { reg3 | 8 } else { reg3 }
    }

    /// Apply REX.X extension to a 3-bit register field.
    #[inline]
    fn extend_x(&self, reg3: u8) -> u8 {
        if self.inst.prefix.rex_x() { reg3 | 8 } else { reg3 }
    }

    /// Resolve the default segment register for a memory operand, honoring
    /// any segment override prefix.
    fn default_segment(&self, base: Option<u8>) -> SegReg {
        if let Some(ov) = self.inst.prefix.seg_override {
            return ov;
        }
        // BP/SP-based addressing defaults to SS; everything else to DS.
        match base {
            Some(4) | Some(5) | Some(12) | Some(13) => SegReg::Ss, // RSP, RBP, R12, R13
            _ => SegReg::Ds,
        }
    }

    /// Decode the r/m operand from a ModR/M byte, handling all addressing
    /// modes including SIB, displacements, and RIP-relative.
    ///
    /// `md` and `rm` are the already-extracted mod and r/m fields (3 bits each,
    /// before REX extension). `size` is the width of the operand.
    fn decode_rm(&mut self, md: u8, rm: u8, size: OperandSize) -> Result<Operand> {
        if md == 3 {
            // Register direct.
            let reg = self.extend_b(rm);
            return Ok(self.gpr_operand(reg, size));
        }

        // Memory operand -- dispatch by address size.
        match self.inst.address_size {
            OperandSize::Word => self.decode_memory_16(md, rm, size),
            OperandSize::Dword | OperandSize::Qword => self.decode_memory_32_64(md, rm, size),
            _ => Err(VmError::GeneralProtection(0)),
        }
    }

    /// Decode a 32/64-bit memory operand (ModR/M + optional SIB + displacement).
    ///
    /// Handles all ModR/M modes including RIP-relative addressing in 64-bit
    /// mode (mod=00, rm=5) and SIB byte (rm=4).
    fn decode_memory_32_64(&mut self, md: u8, rm: u8, size: OperandSize) -> Result<Operand> {
        let is_64 = self.mode == CpuMode::Long64;

        // rm=4 signals SIB byte, rm=5 in mod=00 is special.
        if md == 0 && rm == 5 {
            // mod=00, rm=5: disp32 (32-bit mode) or RIP+disp32 (64-bit mode).
            let disp = self.fetch_u32()? as i32 as i64;
            self.inst.displacement = disp;
            let segment = self.default_segment(None);
            return Ok(Operand::Memory(MemOperand {
                base: None,
                index: None,
                scale: 1,
                displacement: disp,
                segment,
                size,
                rip_relative: is_64,
            }));
        }

        if rm == 4 {
            // SIB byte follows.
            return self.decode_sib(md, size);
        }

        // Regular base register + optional displacement.
        let base_reg = self.extend_b(rm);
        let disp = match md {
            0 => 0i64,
            1 => self.fetch_u8()? as i8 as i64,
            2 => self.fetch_u32()? as i32 as i64,
            _ => unreachable!(),
        };
        self.inst.displacement = disp;
        let segment = self.default_segment(Some(base_reg));
        Ok(Operand::Memory(MemOperand {
            base: Some(base_reg),
            index: None,
            scale: 1,
            displacement: disp,
            segment,
            size,
            rip_relative: false,
        }))
    }

    /// Decode SIB byte and produce a memory operand.
    fn decode_sib(&mut self, md: u8, size: OperandSize) -> Result<Operand> {
        let sib = self.fetch_u8()?;
        self.inst.sib = Some(sib);

        let scale_bits = (sib >> 6) & 3;
        let index3 = (sib >> 3) & 7;
        let base3 = sib & 7;

        let scale = 1u8 << scale_bits;

        // Index register: REX.X extends it. Index = 4 (RSP) means "no index".
        let index_reg = self.extend_x(index3);
        let index = if index3 == 4 && !self.inst.prefix.rex_x() {
            None
        } else {
            Some(index_reg)
        };

        // Base register: REX.B extends it. base3=5 with mod=00 means
        // "no base + disp32".
        let (base, disp) = if base3 == 5 && md == 0 {
            let d = self.fetch_u32()? as i32 as i64;
            (None, d)
        } else {
            let base_reg = self.extend_b(base3);
            let d = match md {
                0 => 0i64,
                1 => self.fetch_u8()? as i8 as i64,
                2 => self.fetch_u32()? as i32 as i64,
                _ => unreachable!(),
            };
            (Some(base_reg), d)
        };

        self.inst.displacement = disp;
        let segment = self.default_segment(base);
        Ok(Operand::Memory(MemOperand {
            base,
            index,
            scale,
            displacement: disp,
            segment,
            size,
            rip_relative: false,
        }))
    }

    /// Decode a 16-bit memory operand (used when address-size is 16-bit).
    ///
    /// 16-bit addressing uses hardcoded register pairs instead of SIB:
    ///
    /// | rm | Base     |
    /// |----|----------|
    /// | 0  | BX + SI  |
    /// | 1  | BX + DI  |
    /// | 2  | BP + SI  |
    /// | 3  | BP + DI  |
    /// | 4  | SI       |
    /// | 5  | DI       |
    /// | 6  | BP (or disp16 if mod=0) |
    /// | 7  | BX       |
    fn decode_memory_16(&mut self, md: u8, rm: u8, size: OperandSize) -> Result<Operand> {
        // Register indices for 16-bit addressing (BX=3, SI=6, DI=7, BP=5).
        let (base, index): (Option<u8>, Option<u8>) = match rm {
            0 => (Some(3), Some(6)), // BX + SI
            1 => (Some(3), Some(7)), // BX + DI
            2 => (Some(5), Some(6)), // BP + SI
            3 => (Some(5), Some(7)), // BP + DI
            4 => (Some(6), None),    // SI
            5 => (Some(7), None),    // DI
            6 => {
                if md == 0 {
                    (None, None) // disp16 only
                } else {
                    (Some(5), None) // BP + disp
                }
            }
            7 => (Some(3), None), // BX
            _ => unreachable!(),
        };

        let disp = if md == 0 && rm == 6 {
            // Direct 16-bit address.
            self.fetch_u16()? as i16 as i64
        } else {
            match md {
                0 => 0i64,
                1 => self.fetch_u8()? as i8 as i64,
                2 => self.fetch_u16()? as i16 as i64,
                _ => unreachable!(),
            }
        };

        self.inst.displacement = disp;
        let segment = self.default_segment(base);
        Ok(Operand::Memory(MemOperand {
            base,
            index,
            scale: 1,
            displacement: disp,
            segment,
            size,
            rip_relative: false,
        }))
    }

    // ===================================================================
    // Operand decoding helpers (reusable patterns)
    // ===================================================================

    /// Decode `operand[0] = r/m, operand[1] = reg` from ModR/M.
    fn decode_modrm_rm_r(&mut self, size: OperandSize) -> Result<()> {
        let modrm = self.fetch_modrm()?;
        let (md, reg, rm) = Self::split_modrm(modrm);
        let reg = self.extend_r(reg);
        let rm_op = self.decode_rm(md, rm, size)?;
        self.set_operand(0, rm_op);
        self.set_operand(1, self.gpr_operand(reg, size));
        self.inst.operand_count = 2;
        Ok(())
    }

    /// Decode `operand[0] = reg, operand[1] = r/m` from ModR/M.
    fn decode_modrm_r_rm(&mut self, size: OperandSize) -> Result<()> {
        let modrm = self.fetch_modrm()?;
        let (md, reg, rm) = Self::split_modrm(modrm);
        let reg = self.extend_r(reg);
        let rm_op = self.decode_rm(md, rm, size)?;
        self.set_operand(0, self.gpr_operand(reg, size));
        self.set_operand(1, rm_op);
        self.inst.operand_count = 2;
        Ok(())
    }

    /// Decode `operand[0] = AL, operand[1] = imm8`.
    fn decode_al_imm8(&mut self) -> Result<()> {
        let imm = self.fetch_u8()? as u64;
        self.inst.immediate = imm;
        self.set_operand(0, self.gpr_operand(0, OperandSize::Byte));
        self.set_operand(1, Operand::Immediate(imm));
        self.inst.operand_count = 2;
        Ok(())
    }

    /// Decode `operand[0] = eAX/rAX, operand[1] = imm` at current operand size.
    fn decode_eax_imm(&mut self) -> Result<()> {
        let sz = self.inst.operand_size;
        // In 64-bit mode, accumulator immediate is still 32-bit (sign-extended).
        let imm_sz = if sz == OperandSize::Qword { OperandSize::Dword } else { sz };
        let imm = self.fetch_imm(imm_sz)?;
        self.inst.immediate = imm;
        self.set_operand(0, self.gpr_operand(0, sz));
        self.set_operand(1, Operand::Immediate(imm));
        self.inst.operand_count = 2;
        Ok(())
    }

    /// Decode a Group instruction: `operand[0] = r/m, operand[1] = imm`.
    ///
    /// The reg field of ModR/M selects the sub-opcode (ADD/OR/ADC/SBB/AND/
    /// SUB/XOR/CMP for Group 1).
    fn decode_group_rm_imm(&mut self, rm_size: OperandSize, imm_size: OperandSize) -> Result<()> {
        let modrm = self.fetch_modrm()?;
        let (md, _reg, rm) = Self::split_modrm(modrm);
        let rm_op = self.decode_rm(md, rm, rm_size)?;
        let imm = self.fetch_imm(imm_size)?;
        self.inst.immediate = imm;
        self.set_operand(0, rm_op);
        self.set_operand(1, Operand::Immediate(imm));
        self.inst.operand_count = 2;
        Ok(())
    }

    /// Decode PUSH/POP register from opcode bits [2:0] + REX.B.
    fn decode_push_pop_reg(&mut self, opcode: u8) -> Result<()> {
        let reg = self.extend_b(opcode & 0x07);
        let sz = self.inst.operand_size;
        self.set_operand(0, self.gpr_operand(reg, sz));
        self.inst.operand_count = 1;
        Ok(())
    }

    /// Decode a relative offset operand (for CALL/JMP/Jcc near).
    ///
    /// The displacement size matches the operand size, except in 64-bit mode
    /// where it is always 32-bit (sign-extended to 64).
    fn decode_rel_operand(&mut self) -> Result<()> {
        let sz = self.inst.operand_size;
        let rel_sz = match self.mode {
            CpuMode::Long64 => OperandSize::Dword,
            _ => {
                if sz == OperandSize::Qword { OperandSize::Dword } else { sz }
            }
        };
        let rel = self.fetch_imm(rel_sz)? as i64;
        self.set_operand(0, Operand::RelativeOffset(rel));
        self.inst.operand_count = 1;
        Ok(())
    }

    /// Decode Group 2 (shift/rotate): r/m, imm8.
    fn decode_shift_imm8(&mut self, size: OperandSize) -> Result<()> {
        let modrm = self.fetch_modrm()?;
        let (md, _reg, rm) = Self::split_modrm(modrm);
        let rm_op = self.decode_rm(md, rm, size)?;
        let imm = self.fetch_u8()? as u64;
        self.inst.immediate = imm;
        self.set_operand(0, rm_op);
        self.set_operand(1, Operand::Immediate(imm));
        self.inst.operand_count = 2;
        Ok(())
    }

    /// Decode Group 2 (shift/rotate): r/m, 1.
    fn decode_shift_one(&mut self, size: OperandSize) -> Result<()> {
        let modrm = self.fetch_modrm()?;
        let (md, _reg, rm) = Self::split_modrm(modrm);
        let rm_op = self.decode_rm(md, rm, size)?;
        self.inst.immediate = 1;
        self.set_operand(0, rm_op);
        self.set_operand(1, Operand::Immediate(1));
        self.inst.operand_count = 2;
        Ok(())
    }

    /// Decode Group 2 (shift/rotate): r/m, CL.
    fn decode_shift_cl(&mut self, size: OperandSize) -> Result<()> {
        let modrm = self.fetch_modrm()?;
        let (md, _reg, rm) = Self::split_modrm(modrm);
        let rm_op = self.decode_rm(md, rm, size)?;
        self.set_operand(0, rm_op);
        self.set_operand(1, self.gpr_operand(1, OperandSize::Byte)); // CL
        self.inst.operand_count = 2;
        Ok(())
    }

    /// Decode Group 3 (TEST/NOT/NEG/MUL/IMUL/DIV/IDIV r/m).
    ///
    /// The reg field of ModR/M selects the sub-opcode. TEST (reg=0,1) has an
    /// immediate; the others do not.
    fn decode_group3(&mut self, size: OperandSize) -> Result<()> {
        let modrm = self.fetch_modrm()?;
        let (md, reg, rm) = Self::split_modrm(modrm);
        let rm_op = self.decode_rm(md, rm, size)?;
        self.set_operand(0, rm_op);

        if reg == 0 || reg == 1 {
            // TEST r/m, imm
            let imm_sz = if size == OperandSize::Qword { OperandSize::Dword } else { size };
            let imm = self.fetch_imm(imm_sz)?;
            self.inst.immediate = imm;
            self.set_operand(1, Operand::Immediate(imm));
            self.inst.operand_count = 2;
        } else {
            // NOT/NEG/MUL/IMUL/DIV/IDIV -- single r/m operand.
            self.inst.operand_count = 1;
        }
        Ok(())
    }

    /// Fetch a memory offset (moffs) whose size is determined by address size.
    fn fetch_moffs(&mut self) -> Result<u64> {
        match self.inst.address_size {
            OperandSize::Word => Ok(self.fetch_u16()? as u64),
            OperandSize::Dword => Ok(self.fetch_u32()? as u64),
            OperandSize::Qword => self.fetch_u64(),
            _ => Err(VmError::GeneralProtection(0)),
        }
    }

    /// Build a memory operand for moffs addressing (direct absolute address).
    fn moffs_operand(&self, addr: u64, size: OperandSize) -> Operand {
        let segment = self.inst.prefix.seg_override.unwrap_or(SegReg::Ds);
        Operand::Memory(MemOperand {
            base: None,
            index: None,
            scale: 1,
            displacement: addr as i64,
            segment,
            size,
            rip_relative: false,
        })
    }

    // ===================================================================
    // Utility helpers
    // ===================================================================

    /// Build a GPR register operand.
    fn gpr_operand(&self, reg: u8, _size: OperandSize) -> Operand {
        Operand::Register(RegOperand::Gpr(reg))
    }

    /// Store an operand at the given index.
    #[inline]
    fn set_operand(&mut self, idx: usize, op: Operand) {
        self.inst.operands[idx] = op;
    }

    /// Return an error if the CPU is in 64-bit mode (for opcodes that are
    /// undefined in long mode).
    fn require_not_64(&self) -> Result<()> {
        if self.mode == CpuMode::Long64 {
            Err(VmError::UndefinedOpcode(self.inst.opcode as u8))
        } else {
            Ok(())
        }
    }
}

// ---------------------------------------------------------------------------
// Static helpers
// ---------------------------------------------------------------------------

/// Returns `true` for opcodes that default to 64-bit operand size in long mode.
///
/// These are PUSH, POP, CALL near, RET near, Jcc near, JMP near, LOOP/JCXZ,
/// and similar control-flow instructions. Intel SDM Vol. 2, Table B-6.
fn is_default_64(opcode: u16) -> bool {
    matches!(
        opcode as u8,
        // PUSH r
        0x50..=0x57 |
        // POP r
        0x58..=0x5F |
        // PUSH imm32
        0x68 |
        // PUSH imm8
        0x6A |
        // CALL near rel32
        0xE8 |
        // JMP near rel32
        0xE9 |
        // JMP short rel8
        0xEB |
        // RET near
        0xC3 |
        // RET near imm16
        0xC2 |
        // RETF
        0xCA | 0xCB |
        // ENTER
        0xC8 |
        // LEAVE
        0xC9 |
        // POP r/m
        0x8F |
        // PUSHF/POPF
        0x9C | 0x9D |
        // LOOP/LOOPE/LOOPNE/JCXZ
        0xE0..=0xE3 |
        // Group 5: INC/DEC/CALL/JMP/PUSH r/m
        0xFF
    )
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::Result;
    use crate::memory::MemoryBus;

    /// Simple flat-memory implementation for testing.
    struct TestMemory {
        data: [u8; 256],
    }

    impl TestMemory {
        fn new(bytes: &[u8]) -> Self {
            let mut data = [0u8; 256];
            data[..bytes.len()].copy_from_slice(bytes);
            TestMemory { data }
        }
    }

    impl MemoryBus for TestMemory {
        fn read_u8(&self, addr: u64) -> Result<u8> {
            Ok(self.data[addr as usize])
        }
        fn read_u16(&self, addr: u64) -> Result<u16> {
            let lo = self.data[addr as usize] as u16;
            let hi = self.data[addr as usize + 1] as u16;
            Ok(lo | (hi << 8))
        }
        fn read_u32(&self, addr: u64) -> Result<u32> {
            let a = addr as usize;
            Ok(u32::from_le_bytes([
                self.data[a],
                self.data[a + 1],
                self.data[a + 2],
                self.data[a + 3],
            ]))
        }
        fn read_u64(&self, addr: u64) -> Result<u64> {
            let a = addr as usize;
            Ok(u64::from_le_bytes([
                self.data[a],
                self.data[a + 1],
                self.data[a + 2],
                self.data[a + 3],
                self.data[a + 4],
                self.data[a + 5],
                self.data[a + 6],
                self.data[a + 7],
            ]))
        }
        fn write_u8(&mut self, _addr: u64, _val: u8) -> Result<()> {
            Ok(())
        }
        fn write_u16(&mut self, _addr: u64, _val: u16) -> Result<()> {
            Ok(())
        }
        fn write_u32(&mut self, _addr: u64, _val: u32) -> Result<()> {
            Ok(())
        }
        fn write_u64(&mut self, _addr: u64, _val: u64) -> Result<()> {
            Ok(())
        }
        fn read_bytes(&self, addr: u64, buf: &mut [u8]) -> Result<()> {
            let a = addr as usize;
            buf.copy_from_slice(&self.data[a..a + buf.len()]);
            Ok(())
        }
        fn write_bytes(&mut self, _addr: u64, _buf: &[u8]) -> Result<()> {
            Ok(())
        }
    }

    #[test]
    fn test_nop() {
        let mem = TestMemory::new(&[0x90]);
        let dec = Decoder::new(CpuMode::Protected32);
        let inst = dec.decode(&mem, 0).unwrap();
        assert_eq!(inst.length, 1);
        assert_eq!(inst.opcode, 0x90);
        assert_eq!(inst.operand_count, 0);
    }

    #[test]
    fn test_add_eax_imm32() {
        // ADD EAX, 0x12345678
        let mem = TestMemory::new(&[0x05, 0x78, 0x56, 0x34, 0x12]);
        let dec = Decoder::new(CpuMode::Protected32);
        let inst = dec.decode(&mem, 0).unwrap();
        assert_eq!(inst.length, 5);
        assert_eq!(inst.opcode, 0x05);
        assert_eq!(inst.operand_count, 2);
        assert_eq!(inst.immediate, 0x12345678);
    }

    #[test]
    fn test_push_reg_64() {
        // PUSH RBX (0x53) in 64-bit mode defaults to 64-bit operand.
        let mem = TestMemory::new(&[0x53]);
        let dec = Decoder::new(CpuMode::Long64);
        let inst = dec.decode(&mem, 0).unwrap();
        assert_eq!(inst.length, 1);
        assert_eq!(inst.operand_size, OperandSize::Qword);
        assert_eq!(inst.operand_count, 1);
    }

    #[test]
    fn test_rex_w_mov_imm64() {
        // REX.W MOV RBX, imm64: 48 BB <8 bytes>
        let mem = TestMemory::new(&[
            0x48, 0xBB, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08,
        ]);
        let dec = Decoder::new(CpuMode::Long64);
        let inst = dec.decode(&mem, 0).unwrap();
        assert_eq!(inst.length, 10);
        assert_eq!(inst.operand_size, OperandSize::Qword);
        assert_eq!(inst.immediate, 0x0807060504030201);
    }

    #[test]
    fn test_jcc_short() {
        // JZ +5 (0x74, 0x05)
        let mem = TestMemory::new(&[0x74, 0x05]);
        let dec = Decoder::new(CpuMode::Protected32);
        let inst = dec.decode(&mem, 0).unwrap();
        assert_eq!(inst.length, 2);
        assert_eq!(inst.operand_count, 1);
        match inst.operands[0] {
            Operand::RelativeOffset(off) => assert_eq!(off, 5),
            _ => panic!("expected RelativeOffset"),
        }
    }

    #[test]
    fn test_modrm_sib() {
        // MOV EAX, [ECX + EDX*4 + 0x10]: 8B 44 91 10
        let mem = TestMemory::new(&[0x8B, 0x44, 0x91, 0x10]);
        let dec = Decoder::new(CpuMode::Protected32);
        let inst = dec.decode(&mem, 0).unwrap();
        assert_eq!(inst.length, 4);
        assert_eq!(inst.operand_count, 2);
        match inst.operands[1] {
            Operand::Memory(ref m) => {
                assert_eq!(m.base, Some(1));  // ECX
                assert_eq!(m.index, Some(2)); // EDX
                assert_eq!(m.scale, 4);
                assert_eq!(m.displacement, 0x10);
            }
            _ => panic!("expected Memory operand"),
        }
    }

    #[test]
    fn test_two_byte_opcode_jcc_near() {
        // JZ near +0x100: 0F 84 00 01 00 00
        let mem = TestMemory::new(&[0x0F, 0x84, 0x00, 0x01, 0x00, 0x00]);
        let dec = Decoder::new(CpuMode::Protected32);
        let inst = dec.decode(&mem, 0).unwrap();
        assert_eq!(inst.length, 6);
        assert_eq!(inst.opcode_map, OpcodeMap::Secondary);
        assert_eq!(inst.opcode, 0x0F84);
        match inst.operands[0] {
            Operand::RelativeOffset(off) => assert_eq!(off, 0x100),
            _ => panic!("expected RelativeOffset"),
        }
    }

    #[test]
    fn test_prefix_66_in_real_mode() {
        // In real mode, 0x66 prefix promotes operand size to 32-bit.
        // ADD EAX, imm32: 66 05 78 56 34 12
        let mem = TestMemory::new(&[0x66, 0x05, 0x78, 0x56, 0x34, 0x12]);
        let dec = Decoder::new(CpuMode::Real16);
        let inst = dec.decode(&mem, 0).unwrap();
        assert_eq!(inst.length, 6);
        assert_eq!(inst.operand_size, OperandSize::Dword);
        assert_eq!(inst.immediate, 0x12345678);
    }

    #[test]
    fn test_rip_relative() {
        // MOV EAX, [RIP + 0x12345678] in 64-bit: 8B 05 78 56 34 12
        let mem = TestMemory::new(&[0x8B, 0x05, 0x78, 0x56, 0x34, 0x12]);
        let dec = Decoder::new(CpuMode::Long64);
        let inst = dec.decode(&mem, 0).unwrap();
        assert_eq!(inst.length, 6);
        match inst.operands[1] {
            Operand::Memory(ref m) => {
                assert!(m.rip_relative);
                assert_eq!(m.displacement, 0x12345678);
                assert!(m.base.is_none());
            }
            _ => panic!("expected Memory operand"),
        }
    }

    #[test]
    fn test_inc_dec_invalid_in_64bit() {
        // INC EAX (0x40) is a REX prefix in 64-bit, not INC.
        // The decoder should treat 0x40 as a REX prefix and decode whatever follows.
        let mem = TestMemory::new(&[0x40, 0x90]); // REX + NOP
        let dec = Decoder::new(CpuMode::Long64);
        let inst = dec.decode(&mem, 0).unwrap();
        // The REX prefix is consumed; NOP is decoded.
        assert_eq!(inst.opcode, 0x90);
        assert_eq!(inst.length, 2);
        assert!(inst.prefix.has_rex());
    }

    #[test]
    fn test_group1_add_rm_imm8() {
        // ADD DWORD [EBX], 0x05: 83 03 05
        let mem = TestMemory::new(&[0x83, 0x03, 0x05]);
        let dec = Decoder::new(CpuMode::Protected32);
        let inst = dec.decode(&mem, 0).unwrap();
        assert_eq!(inst.length, 3);
        assert_eq!(inst.opcode, 0x83);
        assert_eq!(inst.operand_count, 2);
        // reg field of ModR/M (03 >> 3 & 7 = 0) selects ADD.
        assert_eq!(inst.modrm_reg() & 7, 0);
    }

    #[test]
    fn test_movzx() {
        // MOVZX EAX, BYTE [ECX]: 0F B6 01
        let mem = TestMemory::new(&[0x0F, 0xB6, 0x01]);
        let dec = Decoder::new(CpuMode::Protected32);
        let inst = dec.decode(&mem, 0).unwrap();
        assert_eq!(inst.length, 3);
        assert_eq!(inst.opcode_map, OpcodeMap::Secondary);
        assert_eq!(inst.operand_count, 2);
    }

    #[test]
    fn test_16bit_addressing() {
        // In real mode with default 16-bit address: MOV AX, [BX+SI+0x10]
        // 8B 40 10 -- wait, that is 32-bit. Let me use actual 16-bit modrm.
        // MOV AX, [BX+SI+disp8]: 8B 40 10 in 16-bit mode
        // ModR/M 0x40 = mod=01, reg=000(AX), rm=000(BX+SI), disp8=0x10
        let mem = TestMemory::new(&[0x8B, 0x40, 0x10]);
        let dec = Decoder::new(CpuMode::Real16);
        let inst = dec.decode(&mem, 0).unwrap();
        assert_eq!(inst.length, 3);
        assert_eq!(inst.operand_count, 2);
        match inst.operands[1] {
            Operand::Memory(ref m) => {
                assert_eq!(m.base, Some(3));  // BX
                assert_eq!(m.index, Some(6)); // SI
                assert_eq!(m.displacement, 0x10);
            }
            _ => panic!("expected Memory operand"),
        }
    }

    #[test]
    fn test_segment_override() {
        // FS: MOV EAX, [EBX]: 64 8B 03
        let mem = TestMemory::new(&[0x64, 0x8B, 0x03]);
        let dec = Decoder::new(CpuMode::Protected32);
        let inst = dec.decode(&mem, 0).unwrap();
        assert_eq!(inst.prefix.seg_override, Some(SegReg::Fs));
        match inst.operands[1] {
            Operand::Memory(ref m) => {
                assert_eq!(m.segment, SegReg::Fs);
            }
            _ => panic!("expected Memory operand"),
        }
    }

    #[test]
    fn test_ret_imm16() {
        // RET 0x0008: C2 08 00
        let mem = TestMemory::new(&[0xC2, 0x08, 0x00]);
        let dec = Decoder::new(CpuMode::Protected32);
        let inst = dec.decode(&mem, 0).unwrap();
        assert_eq!(inst.length, 3);
        assert_eq!(inst.opcode, 0xC2);
        assert_eq!(inst.immediate, 8);
    }

    #[test]
    fn test_int() {
        // INT 0x80: CD 80
        let mem = TestMemory::new(&[0xCD, 0x80]);
        let dec = Decoder::new(CpuMode::Protected32);
        let inst = dec.decode(&mem, 0).unwrap();
        assert_eq!(inst.length, 2);
        assert_eq!(inst.immediate, 0x80);
    }

    #[test]
    fn test_enter() {
        // ENTER 0x0010, 0x00: C8 10 00 00
        let mem = TestMemory::new(&[0xC8, 0x10, 0x00, 0x00]);
        let dec = Decoder::new(CpuMode::Protected32);
        let inst = dec.decode(&mem, 0).unwrap();
        assert_eq!(inst.length, 4);
        assert_eq!(inst.immediate, 0x10);
        assert_eq!(inst.immediate2, 0);
    }

    #[test]
    fn test_bswap() {
        // BSWAP ECX: 0F C9
        let mem = TestMemory::new(&[0x0F, 0xC9]);
        let dec = Decoder::new(CpuMode::Protected32);
        let inst = dec.decode(&mem, 0).unwrap();
        assert_eq!(inst.length, 2);
        assert_eq!(inst.opcode, 0x0FC9);
        assert_eq!(inst.operand_count, 1);
    }
}
