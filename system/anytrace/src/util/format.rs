//! Hex formatting and register name utilities.

use alloc::string::String;
use alloc::format;

/// Format a u64 as "0x" + zero-padded hex (16 digits).
pub fn hex64(val: u64) -> String {
    // Manual hex formatting since we're no_std
    let mut buf = [0u8; 18]; // "0x" + 16 hex digits
    buf[0] = b'0';
    buf[1] = b'x';
    for i in 0..16 {
        let nibble = ((val >> (60 - i * 4)) & 0xF) as u8;
        buf[2 + i] = if nibble < 10 { b'0' + nibble } else { b'a' + nibble - 10 };
    }
    String::from(core::str::from_utf8(&buf).unwrap_or("0x????????????????"))
}

/// Format a u32 as "0x" + zero-padded hex (8 digits).
pub fn hex32(val: u32) -> String {
    let mut buf = [0u8; 10];
    buf[0] = b'0';
    buf[1] = b'x';
    for i in 0..8 {
        let nibble = ((val >> (28 - i * 4)) & 0xF) as u8;
        buf[2 + i] = if nibble < 10 { b'0' + nibble } else { b'a' + nibble - 10 };
    }
    String::from(core::str::from_utf8(&buf).unwrap_or("0x????????"))
}

/// Format a u8 as zero-padded hex (2 digits, no prefix).
pub fn hex_byte(val: u8) -> [u8; 2] {
    let hi = (val >> 4) & 0xF;
    let lo = val & 0xF;
    [
        if hi < 10 { b'0' + hi } else { b'a' + hi - 10 },
        if lo < 10 { b'0' + lo } else { b'a' + lo - 10 },
    ]
}

/// Format a byte slice as space-separated hex bytes.
pub fn hex_bytes(data: &[u8]) -> String {
    let mut s = String::new();
    for (i, &b) in data.iter().enumerate() {
        if i > 0 { s.push(' '); }
        let h = hex_byte(b);
        s.push(h[0] as char);
        s.push(h[1] as char);
    }
    s
}

/// Register names for the DebugRegs struct (in order).
pub const REG_NAMES: [&str; 19] = [
    "RAX", "RBX", "RCX", "RDX", "RSI", "RDI", "RBP",
    "R8", "R9", "R10", "R11", "R12", "R13", "R14", "R15",
    "RSP", "RIP", "RFLAGS", "CR3",
];

/// Format a u64 as decimal.
pub fn fmt_u64(val: u64) -> String {
    if val == 0 {
        return String::from("0");
    }
    let mut buf = [0u8; 20];
    let mut n = val;
    let mut i = 20;
    while n > 0 {
        i -= 1;
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
    }
    String::from(core::str::from_utf8(&buf[i..]).unwrap_or("?"))
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
