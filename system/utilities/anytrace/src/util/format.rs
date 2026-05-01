//! Hex formatting and register name utilities.

use alloc::string::String;

pub use anyos_std::fmt::{hex32, hex64, hex_byte, hex_bytes};

/// Register names for the DebugRegs struct (in order).
pub const REG_NAMES: [&str; 19] = [
    "RAX", "RBX", "RCX", "RDX", "RSI", "RDI", "RBP", "R8", "R9", "R10", "R11", "R12", "R13", "R14",
    "R15", "RSP", "RIP", "RFLAGS", "CR3",
];

/// Format a u64 as decimal (allocating wrapper around `anyos_std::fmt::fmt_u64`).
pub fn fmt_u64(val: u64) -> String {
    let mut buf = [0u8; 20];
    String::from(anyos_std::fmt::fmt_u64(&mut buf, val))
}

/// Thread state as human-readable string.
pub fn thread_state_str(state: u8) -> &'static str {
    match state {
        0 => "Ready",
        1 => "Running",
        2 => "Blocked",
        3 => "Terminated",
        _ => "Unknown",
    }
}

/// Thread state color (ARGB).
pub fn thread_state_color(state: u8) -> u32 {
    match state {
        0 => 0xFF4CAF50, // Ready = green
        1 => 0xFF2196F3, // Running = blue
        2 => 0xFF9E9E9E, // Blocked = grey
        3 => 0xFFF44336, // Terminated = red
        _ => 0xFFFFFFFF, // Unknown = white
    }
}
