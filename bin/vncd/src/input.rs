//! X11 KeySym → anyOS compositor key encoding, and compositor IPC injection.
//!
//! RFB KeyEvent messages carry an X11 KeySym value.  This module converts
//! those to the `(scancode, char_val, modifiers)` triple expected by
//! `CMD_INJECT_KEY`, and provides `inject_pointer_event` for mouse events.
//!
//! anyOS scancode convention (from `keys.rs` in the compositor):
//!   0x00–0x7F  = USB HID scancode
//!   > 0x7F     = synthetic (special) keys
//!
//! We re-use PC/AT set-2 scancodes for letter/digit keys since that is what
//! the anyOS keyboard driver emits and what `encode_scancode()` expects.

use anyos_std::ipc;

// ── Compositor IPC command codes (mirrors compositor/src/ipc_protocol.rs) ────

/// Compositor channel name.
pub const COMPOSITOR_CHAN_NAME: &str = "compositor";

/// Inject synthetic key event: [CMD, scancode, char_val, is_down, modifiers]
pub const CMD_INJECT_KEY: u32 = 0x1022;

/// Inject synthetic pointer event: [CMD, x, y, buttons_mask, 0]
pub const CMD_INJECT_POINTER: u32 = 0x1023;

// ── anyOS modifier bits ───────────────────────────────────────────────────────

/// Modifier bit: Shift held.
pub const MOD_SHIFT: u32 = 1;
/// Modifier bit: Ctrl held.
pub const MOD_CTRL: u32 = 2;
/// Modifier bit: Alt held.
pub const MOD_ALT: u32 = 4;

// ── State shared across a VNC session ────────────────────────────────────────

/// Tracks which modifier keys are currently pressed so that ordinary key
/// events carry the correct modifier mask.
#[derive(Default, Clone, Copy)]
pub struct ModifierState {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub caps_lock: bool,
}

impl ModifierState {
    /// Build the modifier bitmask from current state.
    pub fn mask(&self) -> u32 {
        let mut m = 0u32;
        if self.shift { m |= MOD_SHIFT; }
        if self.ctrl  { m |= MOD_CTRL; }
        if self.alt   { m |= MOD_ALT; }
        m
    }

    /// Update modifier state when a modifier KeySym is pressed or released.
    /// Returns `true` if the keysym was a modifier (caller should not inject it).
    pub fn update(&mut self, keysym: u32, down: bool) -> bool {
        match keysym {
            // Shift (left & right)
            0xFFE1 | 0xFFE2 => { self.shift = down; true }
            // Ctrl (left & right)
            0xFFE3 | 0xFFE4 => { self.ctrl = down; true }
            // Alt (left) / AltGr (right)
            0xFFE9 | 0xFFEA | 0xFE03 => { self.alt = down; true }
            // Caps Lock (toggle on press only)
            0xFFE5 => {
                if down { self.caps_lock = !self.caps_lock; }
                true
            }
            // Meta/Super/Win keys — ignore
            0xFFEB | 0xFFEC => true,
            _ => false,
        }
    }
}

// ── KeySym mapping ────────────────────────────────────────────────────────────

/// Result of a KeySym conversion.
pub struct KeyMapping {
    /// PC/AT scancode (as used by anyOS `encode_scancode`).
    pub scancode: u32,
    /// Character value (Unicode codepoint or 0 for non-printable).
    pub char_val: u32,
}

/// Convert an X11 KeySym + modifier state to `(scancode, char_val)`.
///
/// Returns `None` for keysyms that should be silently dropped (e.g. dead
/// keys, compose, or anything we don't support yet).
pub fn map_keysym(keysym: u32, mods: &ModifierState) -> Option<KeyMapping> {
    // ── ASCII printable (0x20 Space … 0x7E ~) ─────────────────────────────
    if keysym >= 0x0020 && keysym <= 0x007E {
        let ch = keysym as u8;
        // Map the character to a scancode using the US-QWERTY layout.
        let scancode = char_to_scancode(ch, mods.shift || mods.caps_lock)?;
        return Some(KeyMapping { scancode, char_val: ch as u32 });
    }

    // ── Miscellaneous extended keysyms ────────────────────────────────────
    let (scancode, char_val) = match keysym {
        // Control characters
        0xFF08 => (0x2A, 0x08u32), // BackSpace → scancode 42
        0xFF09 => (0x2B, 0x09),    // Tab → scancode 43
        0xFF0D => (0x28, 0x0D),    // Return/Enter → scancode 40
        0xFF1B => (0x29, 0x1B),    // Escape → scancode 41
        0xFFFF => (0x4C, 0x7F),    // Delete → scancode 76

        // Numpad Enter
        0xFF8D => (0x58, 0x0D),

        // Navigation
        0xFF50 => (0x4A, 0),  // Home
        0xFF57 => (0x4D, 0),  // End
        0xFF55 => (0x4B, 0),  // Page Up
        0xFF56 => (0x4E, 0),  // Page Down
        0xFF63 => (0x49, 0),  // Insert

        // Arrow keys
        0xFF51 => (0x50, 0),  // Left
        0xFF52 => (0x52, 0),  // Up
        0xFF53 => (0x4F, 0),  // Right
        0xFF54 => (0x51, 0),  // Down

        // Function keys F1–F12
        0xFFBE => (0x3A, 0),  // F1
        0xFFBF => (0x3B, 0),  // F2
        0xFFC0 => (0x3C, 0),  // F3
        0xFFC1 => (0x3D, 0),  // F4
        0xFFC2 => (0x3E, 0),  // F5
        0xFFC3 => (0x3F, 0),  // F6
        0xFFC4 => (0x40, 0),  // F7
        0xFFC5 => (0x41, 0),  // F8
        0xFFC6 => (0x42, 0),  // F9
        0xFFC7 => (0x43, 0),  // F10
        0xFFC8 => (0x44, 0),  // F11
        0xFFC9 => (0x45, 0),  // F12

        // Numpad digits (with NumLock — treat as character keys)
        0xFFB0 => (0x62, b'0' as u32), // KP_0
        0xFFB1 => (0x59, b'1' as u32), // KP_1
        0xFFB2 => (0x5A, b'2' as u32), // KP_2
        0xFFB3 => (0x5B, b'3' as u32), // KP_3
        0xFFB4 => (0x5C, b'4' as u32), // KP_4
        0xFFB5 => (0x5D, b'5' as u32), // KP_5
        0xFFB6 => (0x5E, b'6' as u32), // KP_6
        0xFFB7 => (0x5F, b'7' as u32), // KP_7
        0xFFB8 => (0x60, b'8' as u32), // KP_8
        0xFFB9 => (0x61, b'9' as u32), // KP_9
        0xFFAA => (0x55, b'*' as u32), // KP_Multiply
        0xFFAB => (0x57, b'+' as u32), // KP_Add
        0xFFAD => (0x56, b'-' as u32), // KP_Subtract
        0xFFAE => (0x63, b'.' as u32), // KP_Decimal
        0xFFAF => (0x54, b'/' as u32), // KP_Divide

        _ => return None,
    };

    Some(KeyMapping { scancode, char_val })
}

// ── US-QWERTY scancode table ──────────────────────────────────────────────────

/// Map a printable ASCII character to the HID USB scancode that produces it on
/// a standard US-QWERTY keyboard.
///
/// `shifted` reflects whether Shift is held (needed to distinguish e.g. '1'
/// from '!' — both use scancode 0x1E, but different char_val).
fn char_to_scancode(ch: u8, shifted: bool) -> Option<u32> {
    let sc = match ch {
        // Letters (A-Z / a-z share the same scancode)
        b'a' | b'A' => 0x04,
        b'b' | b'B' => 0x05,
        b'c' | b'C' => 0x06,
        b'd' | b'D' => 0x07,
        b'e' | b'E' => 0x08,
        b'f' | b'F' => 0x09,
        b'g' | b'G' => 0x0A,
        b'h' | b'H' => 0x0B,
        b'i' | b'I' => 0x0C,
        b'j' | b'J' => 0x0D,
        b'k' | b'K' => 0x0E,
        b'l' | b'L' => 0x0F,
        b'm' | b'M' => 0x10,
        b'n' | b'N' => 0x11,
        b'o' | b'O' => 0x12,
        b'p' | b'P' => 0x13,
        b'q' | b'Q' => 0x14,
        b'r' | b'R' => 0x15,
        b's' | b'S' => 0x16,
        b't' | b'T' => 0x17,
        b'u' | b'U' => 0x18,
        b'v' | b'V' => 0x19,
        b'w' | b'W' => 0x1A,
        b'x' | b'X' => 0x1B,
        b'y' | b'Y' => 0x1C,
        b'z' | b'Z' => 0x1D,

        // Digits and their shifted symbols
        b'1' | b'!' => 0x1E,
        b'2' | b'@' => 0x1F,
        b'3' | b'#' => 0x20,
        b'4' | b'$' => 0x21,
        b'5' | b'%' => 0x22,
        b'6' | b'^' => 0x23,
        b'7' | b'&' => 0x24,
        b'8' | b'*' => 0x25,
        b'9' | b'(' => 0x26,
        b'0' | b')' => 0x27,

        // Punctuation (unshifted / shifted pairs)
        b'-' | b'_' => 0x2D,
        b'=' | b'+' => 0x2E,
        b'[' | b'{' => 0x2F,
        b']' | b'}' => 0x30,
        b'\\' | b'|' => 0x31,
        b';' | b':' => 0x33,
        b'\'' | b'"' => 0x34,
        b'`' | b'~' => 0x35,
        b',' | b'<' => 0x36,
        b'.' | b'>' => 0x37,
        b'/' | b'?' => 0x38,

        // Space
        b' ' => 0x2C,

        _ => return None,
    };
    let _ = shifted; // shift is reflected in char_val, not the scancode
    Some(sc)
}

// ── Compositor IPC injection ──────────────────────────────────────────────────

/// Send a synthetic key event to the compositor via the event channel.
///
/// `comp_chan` is the compositor's event channel ID (obtained via
/// `ipc::evt_chan_create("compositor")`).
pub fn inject_key(comp_chan: u32, keysym: u32, down: bool, mods: &ModifierState) {
    let mapping = match map_keysym(keysym, mods) {
        Some(m) => m,
        None => return,
    };
    let is_down: u32 = if down { 1 } else { 0 };
    let evt = [
        CMD_INJECT_KEY,
        mapping.scancode,
        mapping.char_val,
        is_down,
        mods.mask(),
    ];
    ipc::evt_chan_emit(comp_chan, &evt);
}

/// Send a synthetic pointer event to the compositor via the event channel.
///
/// `x`, `y` — absolute screen coordinates.
/// `buttons` — RFB button mask: bit 0=left, bit 1=middle, bit 2=right.
pub fn inject_pointer(comp_chan: u32, x: u16, y: u16, buttons: u8) {
    let evt = [
        CMD_INJECT_POINTER,
        x as u32,
        y as u32,
        buttons as u32,
        0,
    ];
    ipc::evt_chan_emit(comp_chan, &evt);
}
