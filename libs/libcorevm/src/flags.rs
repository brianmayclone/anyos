//! RFLAGS computation helpers for x86 arithmetic, logic, and shift operations.
//!
//! Each flag helper is a pure function taking operands and result, returning
//! the new flag bits. This eager-evaluation approach is simpler than lazy flags
//! and fast enough for software emulation.

/// Operand size for flag computation and register access.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperandSize {
    /// 8-bit operand.
    Byte,
    /// 16-bit operand.
    Word,
    /// 32-bit operand.
    Dword,
    /// 64-bit operand.
    Qword,
}

impl OperandSize {
    /// Bit count for this operand size.
    #[inline]
    pub fn bits(self) -> u32 {
        match self {
            OperandSize::Byte => 8,
            OperandSize::Word => 16,
            OperandSize::Dword => 32,
            OperandSize::Qword => 64,
        }
    }

    /// Bitmask for the operand size.
    #[inline]
    pub fn mask(self) -> u64 {
        match self {
            OperandSize::Byte => 0xFF,
            OperandSize::Word => 0xFFFF,
            OperandSize::Dword => 0xFFFF_FFFF,
            OperandSize::Qword => u64::MAX,
        }
    }

    /// Sign bit position for this operand size.
    #[inline]
    pub fn sign_bit(self) -> u64 {
        1u64 << (self.bits() - 1)
    }

    /// Byte count for this operand size.
    #[inline]
    pub fn bytes(self) -> u32 {
        self.bits() / 8
    }
}

// ── RFLAGS bit positions ──

/// Carry flag.
pub const CF: u64 = 1 << 0;
/// Parity flag.
pub const PF: u64 = 1 << 2;
/// Auxiliary carry flag (BCD).
pub const AF: u64 = 1 << 4;
/// Zero flag.
pub const ZF: u64 = 1 << 6;
/// Sign flag.
pub const SF: u64 = 1 << 7;
/// Trap flag (single-step).
pub const TF: u64 = 1 << 8;
/// Interrupt enable flag.
pub const IF: u64 = 1 << 9;
/// Direction flag (string operations).
pub const DF: u64 = 1 << 10;
/// Overflow flag.
pub const OF: u64 = 1 << 11;
/// I/O privilege level (bits 12-13).
pub const IOPL_MASK: u64 = 0x3000;
/// IOPL shift.
pub const IOPL_SHIFT: u32 = 12;
/// Nested task flag.
pub const NT: u64 = 1 << 14;
/// Resume flag.
pub const RF: u64 = 1 << 16;
/// Virtual-8086 mode flag.
pub const VM: u64 = 1 << 17;
/// Alignment check / Access control.
pub const AC: u64 = 1 << 18;
/// Virtual interrupt flag.
pub const VIF: u64 = 1 << 19;
/// Virtual interrupt pending.
pub const VIP: u64 = 1 << 20;
/// CPUID identification flag.
pub const ID: u64 = 1 << 21;

/// Bits that are always set in RFLAGS (bit 1 = 1).
pub const RFLAGS_FIXED: u64 = 0x0002;

/// Mask of the six arithmetic/status flags modified by ALU operations.
pub const ARITH_MASK: u64 = CF | PF | AF | ZF | SF | OF;

// ── Parity lookup table ──

/// PF is set when the low byte of the result has an even number of 1-bits.
/// This 256-byte table precomputes the answer for every possible low byte.
const PARITY_TABLE: [bool; 256] = {
    let mut table = [false; 256];
    let mut i = 0usize;
    while i < 256 {
        let mut bits = 0u32;
        let mut v = i;
        while v > 0 {
            bits += (v & 1) as u32;
            v >>= 1;
        }
        table[i] = (bits & 1) == 0; // even parity = PF set
        i += 1;
    }
    table
};

/// Compute PF (parity flag) from the low byte of `result`.
#[inline]
pub fn parity(result: u64) -> bool {
    PARITY_TABLE[(result & 0xFF) as usize]
}

// ── Flag computation for specific operation classes ──

/// Compute flags for ADD/ADC operation.
///
/// Returns a u64 with only the CF|PF|AF|ZF|SF|OF bits set as appropriate.
/// The caller merges these into RFLAGS via `update_flags()`.
#[inline]
pub fn flags_add(op1: u64, op2: u64, result: u64, size: OperandSize) -> u64 {
    let mask = size.mask();
    let sign = size.sign_bit();
    let res = result & mask;
    let a = op1 & mask;

    let mut f = 0u64;
    // CF: unsigned overflow (result wrapped around)
    if res < a {
        f |= CF;
    }
    // PF: parity of low byte
    if parity(res) {
        f |= PF;
    }
    // AF: carry out of bit 3
    if ((op1 ^ op2 ^ result) & 0x10) != 0 {
        f |= AF;
    }
    // ZF: result is zero
    if res == 0 {
        f |= ZF;
    }
    // SF: sign bit set
    if (res & sign) != 0 {
        f |= SF;
    }
    // OF: signed overflow (both operands same sign, result different sign)
    if ((!(op1 ^ op2) & (op1 ^ result)) & sign) != 0 {
        f |= OF;
    }
    f
}

/// Compute flags for SUB/SBB/CMP operation.
#[inline]
pub fn flags_sub(op1: u64, op2: u64, result: u64, size: OperandSize) -> u64 {
    let mask = size.mask();
    let sign = size.sign_bit();
    let res = result & mask;
    let a = op1 & mask;
    let b = op2 & mask;

    let mut f = 0u64;
    // CF: borrow (op1 < op2 unsigned)
    if a < b {
        f |= CF;
    }
    // PF
    if parity(res) {
        f |= PF;
    }
    // AF: borrow from bit 4
    if ((op1 ^ op2 ^ result) & 0x10) != 0 {
        f |= AF;
    }
    // ZF
    if res == 0 {
        f |= ZF;
    }
    // SF
    if (res & sign) != 0 {
        f |= SF;
    }
    // OF: signed overflow (operands different sign, result sign differs from op1)
    if (((op1 ^ op2) & (op1 ^ result)) & sign) != 0 {
        f |= OF;
    }
    f
}

/// Compute flags for logic operations (AND/OR/XOR/TEST).
///
/// CF and OF are always cleared. AF is undefined (we clear it).
#[inline]
pub fn flags_logic(result: u64, size: OperandSize) -> u64 {
    let mask = size.mask();
    let sign = size.sign_bit();
    let res = result & mask;

    let mut f = 0u64;
    // CF=0, OF=0 (explicitly cleared by not setting them)
    if parity(res) {
        f |= PF;
    }
    if res == 0 {
        f |= ZF;
    }
    if (res & sign) != 0 {
        f |= SF;
    }
    f
}

/// Compute flags for INC operation.
///
/// **CF is NOT modified** — the caller must preserve the old CF.
#[inline]
pub fn flags_inc(op1: u64, result: u64, size: OperandSize) -> u64 {
    let mask = size.mask();
    let sign = size.sign_bit();
    let res = result & mask;

    let mut f = 0u64;
    if parity(res) {
        f |= PF;
    }
    // AF: carry from bit 3
    if ((op1 ^ 1 ^ result) & 0x10) != 0 {
        f |= AF;
    }
    if res == 0 {
        f |= ZF;
    }
    if (res & sign) != 0 {
        f |= SF;
    }
    // OF: signed overflow (op1 was max positive value)
    if (op1 & mask) == (sign - 1) {
        f |= OF;
    }
    f
}

/// Compute flags for DEC operation.
///
/// **CF is NOT modified** — the caller must preserve the old CF.
#[inline]
pub fn flags_dec(op1: u64, result: u64, size: OperandSize) -> u64 {
    let mask = size.mask();
    let sign = size.sign_bit();
    let res = result & mask;

    let mut f = 0u64;
    if parity(res) {
        f |= PF;
    }
    // AF
    if ((op1 ^ 1 ^ result) & 0x10) != 0 {
        f |= AF;
    }
    if res == 0 {
        f |= ZF;
    }
    if (res & sign) != 0 {
        f |= SF;
    }
    // OF: signed overflow (op1 was min negative value, i.e. sign bit set and rest zero)
    if (op1 & mask) == sign {
        f |= OF;
    }
    f
}

/// Compute flags for shift/rotate operations.
///
/// `cf` and `of` are computed by the shift handler and passed in.
/// PF, ZF, SF are derived from the result.
#[inline]
pub fn flags_shift(result: u64, cf: bool, of: bool, size: OperandSize) -> u64 {
    let mask = size.mask();
    let sign = size.sign_bit();
    let res = result & mask;

    let mut f = 0u64;
    if cf {
        f |= CF;
    }
    if parity(res) {
        f |= PF;
    }
    if res == 0 {
        f |= ZF;
    }
    if (res & sign) != 0 {
        f |= SF;
    }
    if of {
        f |= OF;
    }
    f
}

/// Update RFLAGS: clear the arithmetic flags then OR in new computed flags.
///
/// System flags (IF, TF, DF, IOPL, etc.) are preserved.
#[inline]
pub fn update_flags(rflags: &mut u64, new_arith_flags: u64) {
    *rflags = (*rflags & !ARITH_MASK) | (new_arith_flags & ARITH_MASK);
}

/// Update RFLAGS for INC/DEC: same as `update_flags` but preserves CF.
#[inline]
pub fn update_flags_preserve_cf(rflags: &mut u64, new_flags: u64) {
    const INC_DEC_MASK: u64 = PF | AF | ZF | SF | OF;
    *rflags = (*rflags & !INC_DEC_MASK) | (new_flags & INC_DEC_MASK);
}

/// Evaluate a condition code (0-15) against current RFLAGS.
///
/// Used by Jcc, SETcc, CMOVcc instructions.
/// Returns true if the condition is met.
#[inline]
pub fn eval_cc(cc: u8, rflags: u64) -> bool {
    let result = match cc & 0x0E {
        // O: OF=1
        0x00 => (rflags & OF) != 0,
        // B/NAE/C: CF=1
        0x02 => (rflags & CF) != 0,
        // E/Z: ZF=1
        0x04 => (rflags & ZF) != 0,
        // BE/NA: CF=1 or ZF=1
        0x06 => (rflags & (CF | ZF)) != 0,
        // S: SF=1
        0x08 => (rflags & SF) != 0,
        // P/PE: PF=1
        0x0A => (rflags & PF) != 0,
        // L/NGE: SF!=OF
        0x0C => ((rflags & SF) != 0) != ((rflags & OF) != 0),
        // LE/NG: ZF=1 or SF!=OF
        0x0E => (rflags & ZF) != 0 || ((rflags & SF) != 0) != ((rflags & OF) != 0),
        _ => unreachable!(),
    };
    // Odd condition codes are the negation of even ones
    if (cc & 1) != 0 { !result } else { result }
}
