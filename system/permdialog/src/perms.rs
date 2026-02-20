//! Capability bit definitions and permission groups.

// ── Capability bits (must match kernel) ──

const CAP_FILESYSTEM: u32 = 1 << 0;
const CAP_NETWORK: u32    = 1 << 1;
const CAP_AUDIO: u32      = 1 << 2;
const CAP_DISPLAY: u32    = 1 << 3;
const CAP_DEVICE: u32     = 1 << 4;
const CAP_PROCESS: u32    = 1 << 5;
const CAP_COMPOSITOR: u32 = 1 << 9;
const CAP_SYSTEM: u32     = 1 << 10;

// ── Permission groups (user-friendly, grouped) ──

pub struct PermGroup {
    pub mask: u32,
    pub name: &'static str,
    pub desc: &'static str,
}

pub const PERM_GROUPS: &[PermGroup] = &[
    PermGroup {
        mask: CAP_FILESYSTEM,
        name: "Files & Storage",
        desc: "Read and write your files",
    },
    PermGroup {
        mask: CAP_NETWORK,
        name: "Internet & Network",
        desc: "Send and receive data",
    },
    PermGroup {
        mask: CAP_AUDIO,
        name: "Audio",
        desc: "Play sounds and music",
    },
    PermGroup {
        mask: CAP_DISPLAY | CAP_COMPOSITOR,
        name: "Display",
        desc: "Control display and windows",
    },
    PermGroup {
        mask: CAP_DEVICE,
        name: "Devices",
        desc: "Access hardware devices",
    },
    PermGroup {
        mask: CAP_PROCESS | CAP_SYSTEM,
        name: "System",
        desc: "Manage processes and settings",
    },
];

/// Parse a hex string (e.g. "2F") into a u32.
pub fn parse_hex(s: &str) -> u32 {
    let mut val: u32 = 0;
    for &b in s.as_bytes() {
        let digit = match b {
            b'0'..=b'9' => b - b'0',
            b'a'..=b'f' => b - b'a' + 10,
            b'A'..=b'F' => b - b'A' + 10,
            _ => continue,
        };
        val = val.wrapping_shl(4) | digit as u32;
    }
    val
}
