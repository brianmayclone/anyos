//! Basic block detection and representation.
//!
//! A basic block is a straight-line sequence of instructions with a single
//! entry point and one or more exit points (branches, calls, returns, or
//! privileged operations). Caching decoded basic blocks eliminates the
//! per-instruction decode overhead for hot code.

use alloc::vec::Vec;
use crate::decoder::{CpuMode, Decoder};
use crate::error::{Result, VmError};
use crate::instruction::{DecodedInst, OpcodeMap, Operand, RepPrefix};
use crate::memory::MemoryBus;

/// Maximum number of instructions in a single basic block.
///
/// Limits memory usage per block and ensures the interpreter regains control
/// periodically (for interrupt checks, stop requests, etc.).
const MAX_BLOCK_INSTRUCTIONS: usize = 64;

/// Composite key identifying a unique basic block.
///
/// Two instructions at the same physical address but in different CPU modes
/// or with different CS bases decode differently, so all three components
/// are part of the key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct BlockKey {
    /// Physical address of the first instruction byte.
    pub phys_addr: u64,
    /// CPU decode mode at block entry (distinguishes CS.D=0 vs CS.D=1).
    pub mode: CpuMode,
    /// CS segment base at block entry (affects linear address computation).
    pub cs_base: u64,
}

/// A cached sequence of pre-decoded instructions forming a basic block.
#[derive(Clone)]
pub struct BasicBlock {
    /// Pre-decoded instructions in execution order.
    pub instructions: Vec<DecodedInst>,
    /// Total byte length of all instructions in this block.
    pub byte_len: usize,
    /// Whether the block ends with a branch (Jcc, JMP, CALL, RET, etc.).
    pub exits_with_branch: bool,
}

/// Detect and decode a basic block starting at `phys_addr`.
///
/// Scans forward through guest memory, decoding instructions until a block
/// terminator is reached or the maximum block size is hit. Returns the
/// decoded block or an error if the first instruction cannot be decoded.
pub fn detect_basic_block(
    decoder: &Decoder,
    memory: &dyn MemoryBus,
    phys_addr: u64,
) -> Result<BasicBlock> {
    let mut instructions = Vec::new();
    let mut offset: u64 = 0;
    let mut exits_with_branch = false;

    loop {
        if instructions.len() >= MAX_BLOCK_INSTRUCTIONS {
            break;
        }

        let inst = match decoder.decode(memory, phys_addr + offset) {
            Ok(inst) => inst,
            Err(e) => {
                // If we haven't decoded any instructions yet, propagate the error.
                // Otherwise, end the block before the bad instruction.
                if instructions.is_empty() {
                    return Err(e);
                }
                break;
            }
        };

        let is_terminator = is_block_terminator(&inst);
        offset += inst.length as u64;
        instructions.push(inst);

        if is_terminator {
            exits_with_branch = true;
            break;
        }
    }

    Ok(BasicBlock {
        byte_len: offset as usize,
        exits_with_branch,
        instructions,
    })
}

/// Determine whether an instruction ends a basic block.
///
/// Block terminators include unconditional jumps, conditional branches,
/// calls, returns, interrupts, privileged/system instructions, I/O
/// instructions, and REP-prefixed string operations.
fn is_block_terminator(inst: &DecodedInst) -> bool {
    // REP-prefixed instructions are variable-length loops — terminate.
    if inst.rep != RepPrefix::None {
        return true;
    }

    match inst.opcode_map {
        OpcodeMap::Primary => is_primary_terminator(inst),
        OpcodeMap::Secondary => is_secondary_terminator(inst),
        // Escape maps (0F 38, 0F 3A): rarely used, don't terminate.
        _ => false,
    }
}

/// Check primary opcode map (single-byte opcodes) for block terminators.
fn is_primary_terminator(inst: &DecodedInst) -> bool {
    let op = inst.opcode as u8;
    match op {
        // -- Unconditional jumps --
        0xE9 | 0xEB => true, // JMP rel16/32, JMP rel8
        0xEA => true,        // JMP far (ptr16:16/32)

        // -- Conditional branches --
        0x70..=0x7F => true,  // Jcc rel8
        0xE0..=0xE3 => true,  // LOOPNE, LOOPE, LOOP, JCXZ/JECXZ

        // -- Calls and returns --
        0xE8 => true, // CALL rel16/32
        0x9A => true, // CALL far
        0xC2 | 0xC3 => true, // RET imm16, RET
        0xCA | 0xCB => true, // RETF imm16, RETF
        0xCF => true, // IRET/IRETD/IRETQ

        // -- Interrupts --
        0xCC => true, // INT 3
        0xCD => true, // INT imm8
        0xCE => true, // INTO

        // -- System / privileged instructions --
        0xF4 => true, // HLT
        0xFA => true, // CLI  (changes interrupt state)
        0xFB => true, // STI  (changes interrupt state)

        // -- I/O instructions --
        0xE4..=0xE7 => true, // IN AL/AX, imm8; OUT imm8, AL/AX
        0xEC..=0xEF => true, // IN AL/AX, DX; OUT DX, AL/AX
        0x6C..=0x6F => true, // INSB/INSW/OUTSB/OUTSW

        // -- Group FF: indirect JMP/CALL --
        0xFF => {
            let reg = inst.modrm_reg() & 7;
            matches!(reg, 2 | 3 | 4 | 5) // CALL r/m, CALL far, JMP r/m, JMP far
        }

        // -- Group F6/F7: DIV/IDIV can fault (#DE) — terminate for safety --
        0xF6 | 0xF7 => {
            let reg = inst.modrm_reg() & 7;
            matches!(reg, 6 | 7) // DIV, IDIV
        }

        _ => false,
    }
}

/// Check secondary opcode map (0F-prefixed) for block terminators.
fn is_secondary_terminator(inst: &DecodedInst) -> bool {
    let op = inst.opcode as u8;
    match op {
        // -- Conditional branches (0F 80..8F = Jcc rel16/32) --
        0x80..=0x8F => true,

        // -- System instructions --
        0x00 => {
            // Group 6: SLDT, STR, LLDT, LTR, VERR, VERW
            let reg = inst.modrm_reg() & 7;
            matches!(reg, 2 | 3) // LLDT, LTR
        }
        0x01 => {
            // Group 7: SGDT, SIDT, LGDT, LIDT, SMSW, LMSW, INVLPG
            let reg = inst.modrm_reg() & 7;
            matches!(reg, 2 | 3 | 4 | 7) // LGDT, LIDT, SMSW, INVLPG
        }

        // -- MOV to/from control/debug registers --
        0x20..=0x23 => true, // MOV CR, MOV DR

        // -- SYSCALL/SYSRET --
        0x05 | 0x07 => true,

        // -- WRMSR/RDMSR (can change CPU behavior) --
        0x30 | 0x32 => true,

        // -- WBINVD --
        0x09 => true,

        _ => false,
    }
}
