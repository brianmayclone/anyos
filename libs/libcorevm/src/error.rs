//! Error types for libcorevm.
//!
//! `VmError` serves dual purpose: it is both the Rust error type returned
//! from fallible operations and the representation of x86 CPU exceptions.
//! The main execution loop in `cpu.rs` catches these errors and routes
//! them to the guest's IDT as hardware exceptions.

use core::fmt;

/// Errors generated during VM execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VmError {
    /// Attempted to execute an undefined or unsupported opcode (#UD, vector 6).
    UndefinedOpcode(u8),
    /// General protection fault (#GP, vector 13).
    GeneralProtection(u32),
    /// Page fault (#PF, vector 14).
    PageFault { address: u64, error_code: u32 },
    /// Division by zero or overflow (#DE, vector 0).
    DivideByZero,
    /// Stack segment fault (#SS, vector 12).
    StackFault(u32),
    /// Invalid TSS (#TS, vector 10).
    InvalidTss(u32),
    /// Segment not present (#NP, vector 11).
    SegmentNotPresent(u32),
    /// Alignment check (#AC, vector 17).
    AlignmentCheck,
    /// Double fault (#DF, vector 8).
    DoubleFault,
    /// Breakpoint (#BP, vector 3).
    Breakpoint,
    /// Debug exception (#DB, vector 1).
    DebugException,
    /// Overflow (#OF, vector 4).
    Overflow,
    /// Bound range exceeded (#BR, vector 5).
    BoundRange,
    /// x87 FPU error (#MF, vector 16).
    FpuError,
    /// SIMD floating-point exception (#XM, vector 19).
    SimdException,
    /// Guest attempted unsupported I/O on a port with no handler.
    UnhandledIo { port: u16, is_write: bool },
    /// Guest executed HLT — normal exit condition.
    Halted,
    /// Instruction fetch crossed into unmapped memory.
    FetchFault(u64),
    /// Maximum instruction count exceeded (infinite loop protection).
    InstructionLimitExceeded,
    /// Guest memory allocation failed.
    OutOfMemory,
}

impl VmError {
    /// Returns the x86 exception vector number for this error, if applicable.
    pub fn exception_vector(&self) -> Option<u8> {
        match self {
            VmError::DivideByZero => Some(0),
            VmError::DebugException => Some(1),
            VmError::Breakpoint => Some(3),
            VmError::Overflow => Some(4),
            VmError::BoundRange => Some(5),
            VmError::UndefinedOpcode(_) => Some(6),
            VmError::DoubleFault => Some(8),
            VmError::InvalidTss(_) => Some(10),
            VmError::SegmentNotPresent(_) => Some(11),
            VmError::StackFault(_) => Some(12),
            VmError::GeneralProtection(_) => Some(13),
            VmError::PageFault { .. } => Some(14),
            VmError::FpuError => Some(16),
            VmError::AlignmentCheck => Some(17),
            VmError::SimdException => Some(19),
            _ => None,
        }
    }

    /// Returns the error code pushed on the stack for this exception, if any.
    pub fn error_code(&self) -> Option<u32> {
        match self {
            VmError::InvalidTss(ec) => Some(*ec),
            VmError::SegmentNotPresent(ec) => Some(*ec),
            VmError::StackFault(ec) => Some(*ec),
            VmError::GeneralProtection(ec) => Some(*ec),
            VmError::PageFault { error_code, .. } => Some(*error_code),
            VmError::AlignmentCheck => Some(0),
            VmError::DoubleFault => Some(0),
            _ => None,
        }
    }
}

impl fmt::Display for VmError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VmError::UndefinedOpcode(op) => write!(f, "#UD: undefined opcode 0x{:02X}", op),
            VmError::GeneralProtection(ec) => write!(f, "#GP(0x{:04X})", ec),
            VmError::PageFault { address, error_code } => {
                write!(f, "#PF at 0x{:016X} (error=0x{:04X})", address, error_code)
            }
            VmError::DivideByZero => write!(f, "#DE: divide by zero"),
            VmError::StackFault(ec) => write!(f, "#SS(0x{:04X})", ec),
            VmError::InvalidTss(ec) => write!(f, "#TS(0x{:04X})", ec),
            VmError::SegmentNotPresent(ec) => write!(f, "#NP(0x{:04X})", ec),
            VmError::AlignmentCheck => write!(f, "#AC: alignment check"),
            VmError::DoubleFault => write!(f, "#DF: double fault"),
            VmError::Breakpoint => write!(f, "#BP: breakpoint"),
            VmError::DebugException => write!(f, "#DB: debug exception"),
            VmError::Overflow => write!(f, "#OF: overflow"),
            VmError::BoundRange => write!(f, "#BR: bound range exceeded"),
            VmError::FpuError => write!(f, "#MF: x87 FPU error"),
            VmError::SimdException => write!(f, "#XM: SIMD exception"),
            VmError::UnhandledIo { port, is_write } => {
                write!(f, "unhandled I/O {} port 0x{:04X}", if *is_write { "write" } else { "read" }, port)
            }
            VmError::Halted => write!(f, "CPU halted"),
            VmError::FetchFault(addr) => write!(f, "fetch fault at 0x{:016X}", addr),
            VmError::InstructionLimitExceeded => write!(f, "instruction limit exceeded"),
            VmError::OutOfMemory => write!(f, "out of guest memory"),
        }
    }
}

/// Convenience result alias for VM operations.
pub type Result<T> = core::result::Result<T, VmError>;
