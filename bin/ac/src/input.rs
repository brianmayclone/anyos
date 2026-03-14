//! Keyboard input handling for AC (Anywhere Commander).
//!
//! Wraps `con_poll_key` and provides typed `Key` variants that cover
//! all keys used by AC, matching the encoding defined in the kernel's
//! `sys_con_poll_key` implementation.

// ─── Raw key code ranges ─────────────────────────────────────────────────────

const ARROW_PREFIX: u32 = 0x10_0000;
const NAV_PREFIX:   u32 = 0x20_0000;
const FN_PREFIX:    u32 = 0x30_0000;

// ─── Key enum ────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Key {
    // Printable / control
    Char(u8),
    Enter,
    Backspace,
    Tab,
    Escape,
    CtrlC,
    CtrlD,
    // Other Ctrl+letter combos
    Ctrl(u8),   // raw control code 0x01–0x1A

    // Arrow keys
    Up,
    Down,
    Left,
    Right,

    // Navigation
    Home,
    End,
    PageUp,
    PageDown,
    Delete,

    // Function keys
    F1,
    F2,
    F3,
    F4,
    F5,
    F6,
    F7,
    F8,
    F9,
    F10,
    F11,
    F12,

    // Unknown / ignored
    Unknown,
}

// ─── Decoding ────────────────────────────────────────────────────────────────

/// Decode a raw `con_poll_key` return value into a typed [`Key`].
pub fn decode(raw: u32) -> Key {
    if raw == 0 { return Key::Unknown; }

    match raw >> 16 {
        0x10 => match (raw & 0xFF) as u8 {
            b'A' => Key::Up,
            b'B' => Key::Down,
            b'C' => Key::Right,
            b'D' => Key::Left,
            _    => Key::Unknown,
        },
        0x20 => match (raw & 0xFF) as u8 {
            0x48 => Key::Home,
            0x4B => Key::End,
            0x49 => Key::PageUp,
            0x51 => Key::PageDown,
            0x53 => Key::Delete,
            _    => Key::Unknown,
        },
        0x30 => match raw & 0xFF {
            1  => Key::F1,
            2  => Key::F2,
            3  => Key::F3,
            4  => Key::F4,
            5  => Key::F5,
            6  => Key::F6,
            7  => Key::F7,
            8  => Key::F8,
            9  => Key::F9,
            10 => Key::F10,
            11 => Key::F11,
            12 => Key::F12,
            _  => Key::Unknown,
        },
        _ => {
            // Plain character / control code
            let b = (raw & 0xFF) as u8;
            match b {
                0x03 => Key::CtrlC,
                0x04 => Key::CtrlD,
                0x0D | b'\n' => Key::Enter,
                0x08 => Key::Backspace,
                0x09 => Key::Tab,
                0x1B => Key::Escape,
                0x01..=0x1A => Key::Ctrl(b + b'a' - 1), // Ctrl+A = 0x01 → Ctrl('a')
                32..=126    => Key::Char(b),
                _           => Key::Unknown,
            }
        }
    }
}

/// Non-blocking poll — returns `None` if no key is pending.
pub fn poll() -> Option<Key> {
    let raw = anyos_std::sys::con_poll_key();
    if raw == 0 { None } else { Some(decode(raw)) }
}

/// Blocking poll — spins (with yield) until a key arrives.
pub fn read_key() -> Key {
    loop {
        let raw = anyos_std::sys::con_poll_key();
        if raw != 0 { return decode(raw); }
        anyos_std::process::yield_cpu();
    }
}
