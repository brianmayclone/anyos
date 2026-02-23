//! Keyboard driver with scancode set 1 to keycode translation.
//!
//! Processes raw scancodes (from PS/2 IRQ1 or USB-HID) into [`KeyEvent`]s with
//! modifier tracking (Shift, Ctrl, Alt, AltGr, CapsLock) and buffers up to 256
//! events for consumption by the compositor.
//!
//! Character translation is delegated to the [`super::layout`] module, making
//! this driver device-agnostic — both PS/2 and USB-HID feed scancodes here.

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

/// Logical key identifiers translated from scancodes.
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

/// State of keyboard modifier keys.
#[derive(Debug, Clone, Copy, Default)]
pub struct Modifiers {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub altgr: bool,
    pub caps_lock: bool,
}

/// Ring buffer of pending keyboard events.
static KEY_BUFFER: Spinlock<VecDeque<KeyEvent>> = Spinlock::new(VecDeque::new());
static MODIFIERS: Spinlock<Modifiers> = Spinlock::new(Modifiers {
    shift: false,
    ctrl: false,
    alt: false,
    altgr: false,
    caps_lock: false,
});

/// E0 prefix state — persists across IRQ calls.
/// PS/2 sends 0xE0 as a separate byte on one IRQ, then the actual scancode next.
static E0_PENDING: Spinlock<bool> = Spinlock::new(false);

/// Translate a scancode to a Key, considering E0 prefix and modifiers.
fn scancode_to_key(scancode: u8, is_e0: bool, mods: &Modifiers) -> Key {
    // E0-prefixed keys: navigation cluster, right modifiers
    if is_e0 {
        return match scancode {
            0x48 => Key::Up,
            0x50 => Key::Down,
            0x4B => Key::Left,
            0x4D => Key::Right,
            0x47 => Key::Home,
            0x4F => Key::End,
            0x49 => Key::PageUp,
            0x51 => Key::PageDown,
            0x53 => Key::Delete,
            0x1D => Key::RightCtrl,
            0x38 => Key::RightAlt,
            _ => Key::Unknown,
        };
    }

    // Non-E0: special keys first
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
        // Non-E0 nav keys (numpad when NumLock is off)
        0x47 => Key::Home,
        0x48 => Key::Up,
        0x49 => Key::PageUp,
        0x4B => Key::Left,
        0x4D => Key::Right,
        0x4F => Key::End,
        0x50 => Key::Down,
        0x51 => Key::PageDown,
        0x53 => Key::Delete,
        // Character keys: delegate to layout module
        code if code < 128 => {
            let ch = super::layout::translate(code, mods.shift, mods.altgr, mods.caps_lock);
            if ch != '\0' {
                Key::Char(ch)
            } else {
                Key::Unknown
            }
        }
        _ => Key::Unknown,
    }
}

/// Called from IRQ1 handler (or USB-HID driver) to process a keyboard scancode.
///
/// For extended keys, the PS/2 controller sends 0xE0 first (on a separate IRQ),
/// then the actual scancode. USB-HID also sends 0xE0 for Right Alt / Right Ctrl.
pub fn handle_scancode(scancode: u8) {
    // Check and consume E0 prefix
    let mut e0_lock = E0_PENDING.lock();
    if scancode == 0xE0 {
        *e0_lock = true;
        return;
    }
    let is_e0 = *e0_lock;
    *e0_lock = false;
    drop(e0_lock);

    let pressed = scancode & 0x80 == 0;
    let code = scancode & 0x7F;

    // Update modifier state
    let mut mods = MODIFIERS.lock();
    if is_e0 {
        match code {
            0x38 => mods.altgr = pressed,  // E0 0x38 = Right Alt = AltGr
            0x1D => mods.ctrl = pressed,   // E0 0x1D = Right Ctrl
            _ => {}
        }
    } else {
        match code {
            0x2A => mods.shift = pressed,  // Left Shift
            0x36 => mods.shift = pressed,  // Right Shift
            0x1D => mods.ctrl = pressed,   // Left Ctrl
            0x38 => mods.alt = pressed,    // Left Alt (not AltGr)
            0x3A if pressed => mods.caps_lock = !mods.caps_lock,
            _ => {}
        }
    }

    let key = scancode_to_key(code, is_e0, &mods);
    let modifiers = *mods;
    drop(mods);

    let event = KeyEvent {
        scancode: code,
        key,
        pressed,
        modifiers,
    };

    {
        let mut buf = KEY_BUFFER.lock();
        if buf.len() < 256 {
            buf.push_back(event);
        }
    }
    // Wake compositor outside KEY_BUFFER lock to avoid lock ordering issues.
    crate::syscall::handlers::wake_compositor_if_blocked();
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

/// PS/2 keyboard IRQ handler (IRQ 1). Reads scancode from port 0x60.
pub fn irq_handler(_irq: u8) {
    let scancode = unsafe { crate::arch::x86::port::inb(0x60) };
    handle_scancode(scancode);
}
