//! Keyboard layout definitions and scancode-to-character translation.
//!
//! Each layout provides 4 mapping layers indexed by PS/2 scancode (0..128):
//!   - `normal`:      no modifiers
//!   - `shift`:       Shift held
//!   - `altgr`:       AltGr (Right Alt) held
//!   - `shift_altgr`: Shift + AltGr held
//!
//! The active layout is selected at runtime via [`set_layout`] / [`get_layout`].
//! Translation is device-agnostic: both PS/2 and USB-HID keyboards feed
//! scancodes through the same [`translate`] function.

mod us;
mod de;
mod ch;
mod fr;
mod pl;

use crate::sync::spinlock::Spinlock;

/// Layout identifier — each variant corresponds to a static layout definition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum LayoutId {
    UsQwerty = 0,
    DeDe     = 1,
    DeCh     = 2,
    FrFr     = 3,
    PlPl     = 4,
}

/// Metadata about a layout, exposed via SYS_KBD_LIST_LAYOUTS.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct LayoutInfo {
    pub id: u32,
    pub code: [u8; 8],  // e.g. b"en-US\0\0\0"
    pub label: [u8; 4], // e.g. b"US\0\0" (tray icon text)
}

pub const LAYOUT_COUNT: usize = 5;

pub static LAYOUT_INFOS: [LayoutInfo; LAYOUT_COUNT] = [
    LayoutInfo { id: 0, code: *b"en-US\0\0\0", label: *b"US\0\0" },
    LayoutInfo { id: 1, code: *b"de-DE\0\0\0", label: *b"DE\0\0" },
    LayoutInfo { id: 2, code: *b"de-CH\0\0\0", label: *b"CH\0\0" },
    LayoutInfo { id: 3, code: *b"fr-FR\0\0\0", label: *b"FR\0\0" },
    LayoutInfo { id: 4, code: *b"pl-PL\0\0\0", label: *b"PL\0\0" },
];

/// A keyboard layout: 4 layers of 128 chars each, indexed by PS/2 scancode.
pub struct KeyboardLayout {
    pub normal:      [char; 128],
    pub shift:       [char; 128],
    pub altgr:       [char; 128],
    pub shift_altgr: [char; 128],
}

/// Currently active keyboard layout.
static ACTIVE_LAYOUT: Spinlock<LayoutId> = Spinlock::new(LayoutId::UsQwerty);

pub fn set_layout(id: LayoutId) {
    *ACTIVE_LAYOUT.lock() = id;
}

pub fn get_layout() -> LayoutId {
    *ACTIVE_LAYOUT.lock()
}

pub fn layout_id_from_u32(v: u32) -> Option<LayoutId> {
    match v {
        0 => Some(LayoutId::UsQwerty),
        1 => Some(LayoutId::DeDe),
        2 => Some(LayoutId::DeCh),
        3 => Some(LayoutId::FrFr),
        4 => Some(LayoutId::PlPl),
        _ => None,
    }
}

fn layout_from_id(id: LayoutId) -> &'static KeyboardLayout {
    match id {
        LayoutId::UsQwerty => &us::LAYOUT_US,
        LayoutId::DeDe     => &de::LAYOUT_DE,
        LayoutId::DeCh     => &ch::LAYOUT_CH,
        LayoutId::FrFr     => &fr::LAYOUT_FR,
        LayoutId::PlPl     => &pl::LAYOUT_PL,
    }
}

/// Translate a physical keycode (PS/2 scancode, 0..127) to a character
/// using the active layout and current modifier state.
///
/// Returns `'\0'` if no character mapping exists for this key.
pub fn translate(scancode: u8, shift: bool, altgr: bool, caps_lock: bool) -> char {
    let layout = layout_from_id(get_layout());
    let idx = scancode as usize;
    if idx >= 128 {
        return '\0';
    }

    // AltGr takes priority (European convention).
    // If AltGr is held, check the altgr (or shift_altgr) layer first.
    // Fall through to normal/shift if the AltGr mapping is '\0'.
    if altgr {
        let ch = if shift {
            layout.shift_altgr[idx]
        } else {
            layout.altgr[idx]
        };
        if ch != '\0' {
            return ch;
        }
    }

    // CapsLock inverts shift only for alphabetic keys.
    let normal_ch = layout.normal[idx];
    let is_alpha = normal_ch.is_ascii_alphabetic();
    let use_shift = if is_alpha { shift ^ caps_lock } else { shift };

    if use_shift {
        let ch = layout.shift[idx];
        if ch != '\0' {
            return ch;
        }
    }
    normal_ch
}
