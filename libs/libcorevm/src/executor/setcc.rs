//! SETcc instruction handler.
//!
//! SETcc sets a byte operand to 0 or 1 based on the evaluation of a
//! condition code. The condition code is encoded in the low 4 bits of the
//! secondary opcode byte (0F 90 - 0F 9F).

use crate::cpu::Cpu;
use crate::error::Result;
use crate::flags;
use crate::instruction::DecodedInst;
use crate::memory::{GuestMemory, Mmu};

/// SETcc r/m8: set byte to 0 or 1 based on condition code.
///
/// The condition code is taken from the low 4 bits of the opcode.
/// The destination is always an 8-bit operand (r/m8).
///
/// | Opcode | CC  | Condition         |
/// |--------|-----|-------------------|
/// | 0F 90  | O   | OF=1              |
/// | 0F 91  | NO  | OF=0              |
/// | 0F 92  | B   | CF=1              |
/// | 0F 93  | NB  | CF=0              |
/// | 0F 94  | E   | ZF=1              |
/// | 0F 95  | NE  | ZF=0              |
/// | 0F 96  | BE  | CF=1 or ZF=1      |
/// | 0F 97  | NBE | CF=0 and ZF=0     |
/// | 0F 98  | S   | SF=1              |
/// | 0F 99  | NS  | SF=0              |
/// | 0F 9A  | P   | PF=1              |
/// | 0F 9B  | NP  | PF=0              |
/// | 0F 9C  | L   | SF != OF          |
/// | 0F 9D  | NL  | SF == OF          |
/// | 0F 9E  | LE  | ZF=1 or SF != OF  |
/// | 0F 9F  | NLE | ZF=0 and SF == OF |
pub fn exec_setcc(
    cpu: &mut Cpu,
    inst: &DecodedInst,
    memory: &mut GuestMemory,
    mmu: &Mmu,
) -> Result<()> {
    let cc = (inst.opcode as u8) & 0x0F;
    let result = if flags::eval_cc(cc, cpu.regs.rflags) { 1u64 } else { 0u64 };

    // SETcc always writes an 8-bit result. We create a temporary byte-sized
    // instruction context for the write. The decoder should already have set
    // the operand to an 8-bit destination, so we write directly.
    super::write_operand(cpu, inst, &inst.operands[0], result, memory, mmu)?;

    cpu.regs.rip += inst.length as u64;
    Ok(())
}
