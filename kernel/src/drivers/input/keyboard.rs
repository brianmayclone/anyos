//! PS/2 keyboard driver with scancode set 1 to keycode translation.
//!
//! Processes IRQ1 scancodes into [`KeyEvent`]s with modifier tracking (Shift,
//! Ctrl, Alt, CapsLock) and buffers up to 64 events for consumption by the compositor.

use crate::arch::x86::port::inb;
use alloc::collections::VecDeque;
use crate::sync::spinlock::Spinlock;

/// Keyboard event
#[derive(Debug, Clone, Copy)]
pub struct KeyEvent {
    pub scancode: u8,
    pub key: Key,
    pub pressed: bool,
    pub modifiers: Modifiers,
}

/// Logical key identifiers translated from PS/2 scancodes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Char(char),
    Enter,
    Backspace,
    Tab,
    Escape,
    Space,
    Up,
    Down,
    Left,
    Right,
    F1, F2, F3, F4, F5, F6, F7, F8, F9, F10, F11, F12,
    LeftShift,
    RightShift,
    LeftCtrl,
    RightCtrl,
    LeftAlt,
    RightAlt,
    CapsLock,
    Delete,
    Home,
    End,
    PageUp,
    PageDown,
    Unknown,
}

/// State of keyboard modifier keys (Shift, Ctrl, Alt, CapsLock).
#[derive(Debug, Clone, Copy, Default)]
pub struct Modifiers {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub caps_lock: bool,
}

/// Ring buffer of pending keyboard events, capacity 64.
static KEY_BUFFER: Spinlock<VecDeque<KeyEvent>> = Spinlock::new(VecDeque::new());
static MODIFIERS: Spinlock<Modifiers> = Spinlock::new(Modifiers {
    shift: false,
    ctrl: false,
    alt: false,
    caps_lock: false,
});

/// US QWERTY scancode set 1 to ASCII (unshifted)
static SCANCODE_MAP: [char; 128] = {
    let mut map = ['\0'; 128];
    map[0x02] = '1'; map[0x03] = '2'; map[0x04] = '3'; map[0x05] = '4';
    map[0x06] = '5'; map[0x07] = '6'; map[0x08] = '7'; map[0x09] = '8';
    map[0x0A] = '9'; map[0x0B] = '0'; map[0x0C] = '-'; map[0x0D] = '=';
    map[0x10] = 'q'; map[0x11] = 'w'; map[0x12] = 'e'; map[0x13] = 'r';
    map[0x14] = 't'; map[0x15] = 'y'; map[0x16] = 'u'; map[0x17] = 'i';
    map[0x18] = 'o'; map[0x19] = 'p'; map[0x1A] = '['; map[0x1B] = ']';
    map[0x1E] = 'a'; map[0x1F] = 's'; map[0x20] = 'd'; map[0x21] = 'f';
    map[0x22] = 'g'; map[0x23] = 'h'; map[0x24] = 'j'; map[0x25] = 'k';
    map[0x26] = 'l'; map[0x27] = ';'; map[0x28] = '\'';
    map[0x29] = '`'; map[0x2B] = '\\';
    map[0x2C] = 'z'; map[0x2D] = 'x'; map[0x2E] = 'c'; map[0x2F] = 'v';
    map[0x30] = 'b'; map[0x31] = 'n'; map[0x32] = 'm'; map[0x33] = ',';
    map[0x34] = '.'; map[0x35] = '/';
    map
};

/// Shifted version of the scancode map
static SCANCODE_MAP_SHIFT: [char; 128] = {
    let mut map = ['\0'; 128];
    map[0x02] = '!'; map[0x03] = '@'; map[0x04] = '#'; map[0x05] = '$';
    map[0x06] = '%'; map[0x07] = '^'; map[0x08] = '&'; map[0x09] = '*';
    map[0x0A] = '('; map[0x0B] = ')'; map[0x0C] = '_'; map[0x0D] = '+';
    map[0x10] = 'Q'; map[0x11] = 'W'; map[0x12] = 'E'; map[0x13] = 'R';
    map[0x14] = 'T'; map[0x15] = 'Y'; map[0x16] = 'U'; map[0x17] = 'I';
    map[0x18] = 'O'; map[0x19] = 'P'; map[0x1A] = '{'; map[0x1B] = '}';
    map[0x1E] = 'A'; map[0x1F] = 'S'; map[0x20] = 'D'; map[0x21] = 'F';
    map[0x22] = 'G'; map[0x23] = 'H'; map[0x24] = 'J'; map[0x25] = 'K';
    map[0x26] = 'L'; map[0x27] = ':'; map[0x28] = '"';
    map[0x29] = '~'; map[0x2B] = '|';
    map[0x2C] = 'Z'; map[0x2D] = 'X'; map[0x2E] = 'C'; map[0x2F] = 'V';
    map[0x30] = 'B'; map[0x31] = 'N'; map[0x32] = 'M'; map[0x33] = '<';
    map[0x34] = '>'; map[0x35] = '?';
    map
};

fn scancode_to_key(scancode: u8, mods: &Modifiers) -> Key {
    match scancode {
        0x01 => Key::Escape,
        0x0E => Key::Backspace,
        0x0F => Key::Tab,
        0x1C => Key::Enter,
        0x1D => Key::LeftCtrl,
        0x2A => Key::LeftShift,
        0x36 => Key::RightShift,
        0x38 => Key::LeftAlt,
        0x39 => Key::Space,
        0x3A => Key::CapsLock,
        0x3B => Key::F1,
        0x3C => Key::F2,
        0x3D => Key::F3,
        0x3E => Key::F4,
        0x3F => Key::F5,
        0x40 => Key::F6,
        0x41 => Key::F7,
        0x42 => Key::F8,
        0x43 => Key::F9,
        0x44 => Key::F10,
        0x57 => Key::F11,
        0x58 => Key::F12,
        0x47 => Key::Home,
        0x48 => Key::Up,
        0x49 => Key::PageUp,
        0x4B => Key::Left,
        0x4D => Key::Right,
        0x4F => Key::End,
        0x50 => Key::Down,
        0x51 => Key::PageDown,
        0x53 => Key::Delete,
        code if code < 128 => {
            let use_shift = mods.shift ^ mods.caps_lock;
            let ch = if use_shift {
                SCANCODE_MAP_SHIFT[code as usize]
            } else {
                SCANCODE_MAP[code as usize]
            };
            if ch != '\0' {
                Key::Char(ch)
            } else {
                Key::Unknown
            }
        }
        _ => Key::Unknown,
    }
}

/// Called from IRQ1 handler to process a keyboard scancode
pub fn handle_scancode(scancode: u8) {
    let pressed = scancode & 0x80 == 0;
    let code = scancode & 0x7F;

    // Update modifier state
    let mut mods = MODIFIERS.lock();
    match code {
        0x2A => mods.shift = pressed,         // Left shift
        0x36 => mods.shift = pressed,         // Right shift
        0x1D => mods.ctrl = pressed,          // Left ctrl
        0x38 => mods.alt = pressed,           // Left alt
        0x3A if pressed => mods.caps_lock = !mods.caps_lock, // Caps lock toggle
        _ => {}
    }

    let key = scancode_to_key(code, &mods);
    let modifiers = *mods;
    drop(mods);

    let event = KeyEvent {
        scancode: code,
        key,
        pressed,
        modifiers,
    };

    let mut buf = KEY_BUFFER.lock();
    if buf.len() < 64 {
        buf.push_back(event);
    }
}

/// Read a key event from the buffer (non-blocking)
pub fn read_event() -> Option<KeyEvent> {
    KEY_BUFFER.lock().pop_front()
}

/// Check if there are pending key events
pub fn has_event() -> bool {
    !KEY_BUFFER.lock().is_empty()
}

/// Get current modifier state
pub fn modifiers() -> Modifiers {
    *MODIFIERS.lock()
}
