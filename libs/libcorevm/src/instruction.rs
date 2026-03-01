//! Decoded x86 instruction representation.
//!
//! The decoder produces a `DecodedInst` struct that fully describes the
//! instruction: opcode, operands, prefix state, and sizes. The executor
//! consumes this struct to carry out the operation.

use crate::flags::OperandSize;
use crate::registers::SegReg;

/// An x86 instruction fully decoded from its byte encoding.
#[derive(Debug, Clone)]
pub struct DecodedInst {
    /// Length of the encoded instruction in bytes (1-15).
    pub length: u8,

    /// Primary opcode byte (after any escape bytes).
    /// For two-byte opcodes (0F xx), stored as 0x0F00 | byte2.
    pub opcode: u16,

    /// Which opcode map this instruction belongs to.
    pub opcode_map: OpcodeMap,

    /// Operand size (determined by mode + prefixes + REX.W).
    pub operand_size: OperandSize,

    /// Address size for memory operands.
    pub address_size: OperandSize,

    /// Decoded operands (up to 3 for x86).
    pub operands: [Operand; 3],

    /// Number of valid operands.
    pub operand_count: u8,

    /// Prefix state (segment override, size overrides, LOCK, REX).
    pub prefix: PrefixState,

    /// ModR/M byte if present.
    pub modrm: Option<u8>,

    /// SIB byte if present.
    pub sib: Option<u8>,

    /// Displacement value (sign-extended to i64).
    pub displacement: i64,

    /// Immediate value.
    pub immediate: u64,

    /// Second immediate (for ENTER, far pointers, etc.).
    pub immediate2: u64,

    /// REP/REPNE prefix for string operations.
    pub rep: RepPrefix,
}

impl DecodedInst {
    /// Create a zeroed instruction (used by decoder as starting point).
    pub fn empty() -> Self {
        DecodedInst {
            length: 0,
            opcode: 0,
            opcode_map: OpcodeMap::Primary,
            operand_size: OperandSize::Dword,
            address_size: OperandSize::Dword,
            operands: [Operand::None, Operand::None, Operand::None],
            operand_count: 0,
            prefix: PrefixState::default(),
            modrm: None,
            sib: None,
            displacement: 0,
            immediate: 0,
            immediate2: 0,
            rep: RepPrefix::None,
        }
    }

    /// Get the ModR/M reg field (bits [5:3]), including REX.R extension.
    #[inline]
    pub fn modrm_reg(&self) -> u8 {
        let base = self.modrm.map(|m| (m >> 3) & 7).unwrap_or(0);
        if self.prefix.rex_r() { base | 8 } else { base }
    }

    /// Get the ModR/M r/m field (bits [2:0]), including REX.B extension.
    #[inline]
    pub fn modrm_rm(&self) -> u8 {
        let base = self.modrm.map(|m| m & 7).unwrap_or(0);
        if self.prefix.rex_b() { base | 8 } else { base }
    }

    /// Get the ModR/M mod field (bits [7:6]).
    #[inline]
    pub fn modrm_mod(&self) -> u8 {
        self.modrm.map(|m| (m >> 6) & 3).unwrap_or(0)
    }
}

/// Opcode map identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpcodeMap {
    /// One-byte opcodes (no escape prefix).
    Primary,
    /// Two-byte opcodes (0F xx).
    Secondary,
    /// Three-byte opcodes (0F 38 xx).
    Escape0F38,
    /// Three-byte opcodes (0F 3A xx).
    Escape0F3A,
}

/// Decoded prefix state.
#[derive(Debug, Clone, Copy, Default)]
pub struct PrefixState {
    /// Segment override (None = use default segment).
    pub seg_override: Option<SegReg>,
    /// Operand-size override (0x66 prefix).
    pub operand_size_override: bool,
    /// Address-size override (0x67 prefix).
    pub address_size_override: bool,
    /// LOCK prefix (0xF0).
    pub lock: bool,
    /// REX prefix byte (0 if no REX). Bits: [3]=W, [2]=R, [1]=X, [0]=B.
    pub rex: u8,
}

impl PrefixState {
    /// REX.W bit — promotes operand size to 64-bit.
    #[inline]
    pub fn rex_w(&self) -> bool {
        self.rex & 0x08 != 0
    }

    /// REX.R bit — extends ModR/M reg field to 4 bits.
    #[inline]
    pub fn rex_r(&self) -> bool {
        self.rex & 0x04 != 0
    }

    /// REX.X bit — extends SIB index field to 4 bits.
    #[inline]
    pub fn rex_x(&self) -> bool {
        self.rex & 0x02 != 0
    }

    /// REX.B bit — extends ModR/M r/m, SIB base, or opcode reg field.
    #[inline]
    pub fn rex_b(&self) -> bool {
        self.rex & 0x01 != 0
    }

    /// Whether any REX prefix is present (changes 8-bit register encoding).
    #[inline]
    pub fn has_rex(&self) -> bool {
        self.rex != 0
    }
}

/// REP prefix type for string operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepPrefix {
    /// No REP prefix.
    None,
    /// REP/REPE prefix (0xF3).
    Rep,
    /// REPNE prefix (0xF2).
    Repne,
}

impl Default for RepPrefix {
    fn default() -> Self {
        RepPrefix::None
    }
}

/// An instruction operand.
#[derive(Debug, Clone, Copy)]
pub enum Operand {
    /// Unused operand slot.
    None,
    /// Register operand.
    Register(RegOperand),
    /// Memory operand (address from ModR/M + SIB + displacement).
    Memory(MemOperand),
    /// Immediate value.
    Immediate(u64),
    /// Relative offset (for JMP/CALL/Jcc — sign-extended displacement).
    RelativeOffset(i64),
    /// Far pointer (segment:offset).
    FarPointer { segment: u16, offset: u64 },
}

/// Register operand sub-types.
#[derive(Debug, Clone, Copy)]
pub enum RegOperand {
    /// General-purpose register (0-15, with REX extension applied).
    Gpr(u8),
    /// Segment register.
    Seg(SegReg),
    /// Control register (CR0-CR4, CR8).
    Cr(u8),
    /// Debug register (DR0-DR7).
    Dr(u8),
    /// XMM register (0-15).
    Xmm(u8),
    /// x87 FPU register ST(0)-ST(7).
    Fpu(u8),
}

/// Memory operand (effective address components).
#[derive(Debug, Clone, Copy)]
pub struct MemOperand {
    /// Base register index (None = no base register).
    pub base: Option<u8>,
    /// Index register index (None = no index register).
    pub index: Option<u8>,
    /// Scale factor (1, 2, 4, or 8).
    pub scale: u8,
    /// Displacement (sign-extended).
    pub displacement: i64,
    /// Segment register used for this memory access.
    pub segment: SegReg,
    /// Width of the memory access.
    pub size: OperandSize,
    /// RIP-relative addressing (64-bit mode only).
    pub rip_relative: bool,
}
