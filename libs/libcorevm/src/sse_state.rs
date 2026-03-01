//! SSE/SSE2 register state (XMM0-XMM15 and MXCSR).
//!
//! Provides the [`SseState`] struct holding all 16 XMM registers and the
//! MXCSR control/status register. Each [`Xmm`] register is 128 bits stored
//! as two `u64` halves, with convenience methods for reinterpreting the
//! contents as packed `f32`, `f64`, or `u32` lanes.

// ── MXCSR bit positions ──

/// Invalid Operation exception flag (bit 0).
pub const MXCSR_IE: u32 = 0;
/// Denormal Operand exception flag (bit 1).
pub const MXCSR_DE: u32 = 1;
/// Divide-by-Zero exception flag (bit 2).
pub const MXCSR_ZE: u32 = 2;
/// Overflow exception flag (bit 3).
pub const MXCSR_OE: u32 = 3;
/// Underflow exception flag (bit 4).
pub const MXCSR_UE: u32 = 4;
/// Precision exception flag (bit 5).
pub const MXCSR_PE: u32 = 5;
/// Denormals Are Zeros mode (bit 6).
pub const MXCSR_DAZ: u32 = 6;
/// Invalid Operation exception mask (bit 7).
pub const MXCSR_IM: u32 = 7;
/// Denormal Operand exception mask (bit 8).
pub const MXCSR_DM: u32 = 8;
/// Divide-by-Zero exception mask (bit 9).
pub const MXCSR_ZM: u32 = 9;
/// Overflow exception mask (bit 10).
pub const MXCSR_OM: u32 = 10;
/// Underflow exception mask (bit 11).
pub const MXCSR_UM: u32 = 11;
/// Precision exception mask (bit 12).
pub const MXCSR_PM: u32 = 12;
/// Rounding Control low bit (bit 13). Bits [14:13] together encode the mode:
/// - 00 = round to nearest (even)
/// - 01 = round toward negative infinity
/// - 10 = round toward positive infinity
/// - 11 = round toward zero (truncate)
pub const MXCSR_RC: u32 = 13;
/// Flush to Zero mode (bit 15).
pub const MXCSR_FZ: u32 = 15;

/// Default MXCSR value after `LDMXCSR` reset / power-on.
///
/// All exception flags cleared, all exception masks set (bits 7-12),
/// round-to-nearest mode, DAZ and FZ both off.
pub const MXCSR_DEFAULT: u32 = 0x1F80;

/// Mask of all writable MXCSR bits (bits 0-15). Bits 16-31 are reserved
/// and must be zero; writing a set reserved bit causes `#GP`.
pub const MXCSR_WRITE_MASK: u32 = 0xFFFF;

/// A 128-bit SSE register stored as two 64-bit halves.
///
/// `lo` holds bits [63:0] and `hi` holds bits [127:64].
#[derive(Debug, Clone, Copy, Default)]
#[repr(C, align(16))]
pub struct Xmm {
    /// Lower 64 bits (bits [63:0]).
    pub lo: u64,
    /// Upper 64 bits (bits [127:64]).
    pub hi: u64,
}

impl Xmm {
    /// Construct an XMM register from four packed `u32` values.
    ///
    /// `vals[0]` occupies bits [31:0], `vals[1]` bits [63:32],
    /// `vals[2]` bits [95:64], `vals[3]` bits [127:96].
    #[inline]
    pub fn from_u32x4(vals: [u32; 4]) -> Self {
        Xmm {
            lo: (vals[0] as u64) | ((vals[1] as u64) << 32),
            hi: (vals[2] as u64) | ((vals[3] as u64) << 32),
        }
    }

    /// Extract four packed `u32` values from the register.
    ///
    /// Returns `[bits[31:0], bits[63:32], bits[95:64], bits[127:96]]`.
    #[inline]
    pub fn to_u32x4(self) -> [u32; 4] {
        [
            self.lo as u32,
            (self.lo >> 32) as u32,
            self.hi as u32,
            (self.hi >> 32) as u32,
        ]
    }

    /// Construct an XMM register from four packed single-precision floats.
    ///
    /// `vals[0]` occupies bits [31:0] (lowest element), `vals[3]` bits [127:96].
    #[inline]
    pub fn from_f32x4(vals: [f32; 4]) -> Self {
        Self::from_u32x4([
            vals[0].to_bits(),
            vals[1].to_bits(),
            vals[2].to_bits(),
            vals[3].to_bits(),
        ])
    }

    /// Extract four packed single-precision floats from the register.
    #[inline]
    pub fn to_f32x4(self) -> [f32; 4] {
        let u = self.to_u32x4();
        [
            f32::from_bits(u[0]),
            f32::from_bits(u[1]),
            f32::from_bits(u[2]),
            f32::from_bits(u[3]),
        ]
    }

    /// Construct an XMM register from two packed double-precision floats.
    ///
    /// `vals[0]` occupies bits [63:0] (low element), `vals[1]` bits [127:64].
    #[inline]
    pub fn from_f64x2(vals: [f64; 2]) -> Self {
        Xmm {
            lo: vals[0].to_bits(),
            hi: vals[1].to_bits(),
        }
    }

    /// Extract two packed double-precision floats from the register.
    #[inline]
    pub fn to_f64x2(self) -> [f64; 2] {
        [f64::from_bits(self.lo), f64::from_bits(self.hi)]
    }
}

/// SSE register state: 16 XMM registers and the MXCSR control/status register.
pub struct SseState {
    /// XMM registers 0-15.
    pub xmm: [Xmm; 16],

    /// MXCSR (SSE control/status register).
    ///
    /// Bits [5:0] are exception status flags, bits [12:7] are exception
    /// masks, bits [14:13] are rounding control, bit 6 is DAZ, bit 15 is FZ.
    pub mxcsr: u32,
}

impl SseState {
    /// Create a new SSE state with all XMM registers zeroed and MXCSR set
    /// to the default value (0x1F80: all exceptions masked, round to nearest).
    pub fn new() -> Self {
        SseState {
            xmm: [Xmm::default(); 16],
            mxcsr: MXCSR_DEFAULT,
        }
    }
}
