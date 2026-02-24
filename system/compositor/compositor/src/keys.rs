//! Keyboard scancode → key code translation.
//!
//! Converts raw PS/2 scancodes from the kernel's `input_poll()` into the
//! `KEY_*` constants that userspace apps (uisys_client) expect.

// ── Key Codes (must match uisys_client/src/types.rs) ─────────────────────────

pub const KEY_ENTER: u32     = 0x100;
pub const KEY_BACKSPACE: u32 = 0x101;
pub const KEY_TAB: u32       = 0x102;
pub const KEY_ESCAPE: u32    = 0x103;
pub const KEY_SPACE: u32     = 0x104;
pub const KEY_UP: u32        = 0x105;
pub const KEY_DOWN: u32      = 0x106;
pub const KEY_LEFT: u32      = 0x107;
pub const KEY_RIGHT: u32     = 0x108;

pub const KEY_DELETE: u32    = 0x120;
pub const KEY_HOME: u32      = 0x121;
pub const KEY_END: u32       = 0x122;
pub const KEY_PAGE_UP: u32   = 0x123;
pub const KEY_PAGE_DOWN: u32 = 0x124;

pub const KEY_F1: u32  = 0x140;
pub const KEY_F2: u32  = 0x141;
pub const KEY_F3: u32  = 0x142;
pub const KEY_F4: u32  = 0x143;
pub const KEY_F5: u32  = 0x144;
pub const KEY_F6: u32  = 0x145;
pub const KEY_F7: u32  = 0x146;
pub const KEY_F8: u32  = 0x147;
pub const KEY_F9: u32  = 0x148;
pub const KEY_F10: u32 = 0x149;
pub const KEY_F11: u32 = 0x14A;
pub const KEY_F12: u32 = 0x14B;

pub const KEY_VOLUME_UP: u32   = 0x160;
pub const KEY_VOLUME_DOWN: u32 = 0x161;
pub const KEY_VOLUME_MUTE: u32 = 0x162;

// ── Scancode Translation ─────────────────────────────────────────────────────

/// Translate a raw PS/2 scancode into a `KEY_*` code.
///
/// Special keys (Enter, arrows, etc.) are mapped to the `KEY_*` constants above.
/// Regular character keys pass through unchanged — apps use the `chr` field for those.
pub fn encode_scancode(scancode: u32) -> u32 {
    match scancode {
        0x1C => KEY_ENTER,
        0x0E => KEY_BACKSPACE,
        0x0F => KEY_TAB,
        0x01 => KEY_ESCAPE,
        0x39 => KEY_SPACE,
        0x48 => KEY_UP,
        0x50 => KEY_DOWN,
        0x4B => KEY_LEFT,
        0x4D => KEY_RIGHT,
        0x53 => KEY_DELETE,
        0x47 => KEY_HOME,
        0x4F => KEY_END,
        0x49 => KEY_PAGE_UP,
        0x51 => KEY_PAGE_DOWN,
        // F-keys
        0x3B => KEY_F1,
        0x3C => KEY_F2,
        0x3D => KEY_F3,
        0x3E => KEY_F4,
        0x3F => KEY_F5,
        0x40 => KEY_F6,
        0x41 => KEY_F7,
        0x42 => KEY_F8,
        0x43 => KEY_F9,
        0x44 => KEY_F10,
        0x57 => KEY_F11,
        0x58 => KEY_F12,
        // Multimedia keys (virtual scancodes from kernel, E0-prefixed)
        0x130 => KEY_VOLUME_UP,
        0x12E => KEY_VOLUME_DOWN,
        0x120 => KEY_VOLUME_MUTE,
        // All other scancodes pass through (apps use chr value)
        other => other,
    }
}
