//! x87 FPU register file and state.
//!
//! Models the x87 floating-point unit: eight 80-bit data registers (stored
//! as `f64` with accepted precision loss for emulation), a tag word, control
//! word, status word, and the last-instruction/last-data pointers. The stack
//! is managed via the `top` pointer with push/pop semantics matching the
//! hardware's wrap-around behavior.

/// Tag word values for each physical FPU register (2 bits per register).
pub const TAG_VALID: u8 = 0b00;
/// Tag indicating the register contains zero.
pub const TAG_ZERO: u8 = 0b01;
/// Tag indicating the register contains a special value (NaN, infinity, denormal).
pub const TAG_SPECIAL: u8 = 0b10;
/// Tag indicating the register is empty (available for push).
pub const TAG_EMPTY: u8 = 0b11;

/// x87 FPU register file and associated control state.
///
/// The eight data registers are organized as a circular stack addressed
/// relative to `top`. `ST(0)` is the physical register at index `top`,
/// `ST(1)` is at `(top + 1) % 8`, and so on.
pub struct FpuState {
    /// x87 data registers ST(0)-ST(7), stored as `f64`.
    ///
    /// The hardware uses 80-bit extended precision internally; this
    /// implementation accepts the precision reduction to 64-bit `f64`
    /// which is sufficient for most emulated guest software.
    pub st: [f64; 8],

    /// Top-of-stack pointer (0-7).
    ///
    /// Points to the physical register that is currently `ST(0)`.
    pub top: u8,

    /// FPU control word.
    ///
    /// Default 0x037F: all exceptions masked, double precision (53-bit
    /// significand), round to nearest even.
    pub fcw: u16,

    /// FPU status word.
    ///
    /// Contains condition codes (C0-C3), exception flags, TOP field
    /// (bits [13:11]), and busy flag.
    pub fsw: u16,

    /// Tag word (2 bits per register, 16 bits total).
    ///
    /// Bits [2*i+1 : 2*i] hold the tag for physical register `i`:
    /// - 00 = valid
    /// - 01 = zero
    /// - 10 = special (NaN, infinity, denormal, unsupported)
    /// - 11 = empty
    pub ftw: u16,

    /// Last FPU instruction pointer (linear address of the last x87 instruction).
    pub fip: u64,

    /// Last FPU data pointer (linear address of the last x87 memory operand).
    pub fdp: u64,

    /// Last FPU opcode (lower 11 bits of the last x87 instruction's first two bytes).
    pub fop: u16,
}

impl FpuState {
    /// Create a new FPU state matching the hardware `FINIT` / power-on defaults.
    ///
    /// - FCW = 0x037F (all exceptions masked, double precision, round nearest)
    /// - FSW = 0x0000 (no exceptions, no condition codes, TOP=0)
    /// - FTW = 0xFFFF (all registers tagged empty)
    /// - All data registers, pointers, and opcode zeroed.
    pub fn new() -> Self {
        FpuState {
            st: [0.0f64; 8],
            top: 0,
            fcw: 0x037F,
            fsw: 0x0000,
            ftw: 0xFFFF,
            fip: 0,
            fdp: 0,
            fop: 0,
        }
    }

    /// Push a value onto the FPU stack.
    ///
    /// Decrements `top` (mod 8), stores `val` at the new `ST(0)`, and
    /// updates the tag word for that register based on the value.
    pub fn push(&mut self, val: f64) {
        self.top = self.top.wrapping_sub(1) & 7;
        self.st[self.top as usize] = val;

        let tag = classify_tag(val);
        self.set_tag(self.top, tag);

        // Update the TOP field in FSW (bits [13:11]).
        self.fsw = (self.fsw & !0x3800) | ((self.top as u16) << 11);
    }

    /// Pop a value from the FPU stack.
    ///
    /// Reads `ST(0)`, marks the physical register as empty, then
    /// increments `top` (mod 8). Returns the popped value.
    pub fn pop(&mut self) -> f64 {
        let val = self.st[self.top as usize];
        self.set_tag(self.top, TAG_EMPTY);
        self.top = (self.top + 1) & 7;

        // Update the TOP field in FSW.
        self.fsw = (self.fsw & !0x3800) | ((self.top as u16) << 11);

        val
    }

    /// Read `ST(i)` relative to the current top of stack.
    ///
    /// `ST(0)` is the register at `top`, `ST(1)` is at `(top+1) % 8`, etc.
    #[inline]
    pub fn st(&self, i: u8) -> f64 {
        let phys = (self.top.wrapping_add(i)) & 7;
        self.st[phys as usize]
    }

    /// Write `ST(i)` relative to the current top of stack.
    ///
    /// Also updates the tag word for the targeted physical register.
    pub fn set_st(&mut self, i: u8, val: f64) {
        let phys = (self.top.wrapping_add(i)) & 7;
        self.st[phys as usize] = val;
        let tag = classify_tag(val);
        self.set_tag(phys, tag);
    }

    /// Set the 2-bit tag for a physical register (0-7).
    ///
    /// `tag` must be one of [`TAG_VALID`], [`TAG_ZERO`], [`TAG_SPECIAL`],
    /// or [`TAG_EMPTY`].
    #[inline]
    pub fn set_tag(&mut self, reg: u8, tag: u8) {
        let shift = (reg & 7) * 2;
        self.ftw = (self.ftw & !(0x03 << shift)) | (((tag & 0x03) as u16) << shift);
    }

    /// Read the 2-bit tag for a physical register (0-7).
    #[inline]
    pub fn get_tag(&self, reg: u8) -> u8 {
        let shift = (reg & 7) * 2;
        ((self.ftw >> shift) & 0x03) as u8
    }

    /// Reset the FPU state to power-on / `FINIT` defaults.
    ///
    /// Equivalent to constructing a new [`FpuState`] via [`FpuState::new()`].
    pub fn reset(&mut self) {
        *self = Self::new();
    }
}

/// Classify an `f64` value into the appropriate x87 tag.
///
/// - Zero (positive or negative) maps to [`TAG_ZERO`].
/// - NaN and infinity map to [`TAG_SPECIAL`].
/// - All other finite values map to [`TAG_VALID`].
fn classify_tag(val: f64) -> u8 {
    if val == 0.0 {
        TAG_ZERO
    } else if val.is_nan() || val.is_infinite() {
        TAG_SPECIAL
    } else {
        TAG_VALID
    }
}
