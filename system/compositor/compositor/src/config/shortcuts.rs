//! Configurable keyboard shortcuts.

use crate::keys::*;

use super::file::{read_string, register_manifest};

const MAX_SHORTCUTS: usize = 32;

#[derive(Clone)]
pub struct KeyboardShortcut {
    pub modifiers: u8,
    pub key_code: u32,
    pub action: ShortcutAction,
}

#[derive(Clone)]
pub enum ShortcutAction {
    Launch(alloc::string::String),
    ShowDesktop,
    TileWindows,
    LockScreen,
}

pub fn read_shortcuts() -> alloc::vec::Vec<KeyboardShortcut> {
    register_manifest();
    let text = match read_string("shortcuts/mappings_blob") {
        Some(t) => t,
        None => return alloc::vec::Vec::new(),
    };

    let mut shortcuts = alloc::vec::Vec::with_capacity(8);

    for line in text.split('\n') {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if shortcuts.len() >= MAX_SHORTCUTS {
            break;
        }
        if let Some(eq_pos) = line.find('=') {
            let combo = line[..eq_pos].trim();
            let action_str = line[eq_pos + 1..].trim();
            if combo.is_empty() || action_str.is_empty() {
                continue;
            }
            if let Some(sc) = parse_shortcut(combo, action_str) {
                shortcuts.push(sc);
            }
        }
    }

    shortcuts
}

fn parse_shortcut(combo: &str, action_str: &str) -> Option<KeyboardShortcut> {
    let mut mods: u8 = 0;
    let mut key_code: Option<u32> = None;

    for part in combo.split('+') {
        let part = part.trim();
        match part {
            "Shift" => mods |= 1,
            "Ctrl" | "Control" => mods |= 2,
            "Alt" => mods |= 4,
            "Super" | "Win" | "Meta" => mods |= 8,
            _ => key_code = parse_key_name(part),
        }
    }

    let kc = key_code?;
    let action = match action_str {
        "show_desktop" => ShortcutAction::ShowDesktop,
        "tile_windows" => ShortcutAction::TileWindows,
        "lock_screen" => ShortcutAction::LockScreen,
        path => ShortcutAction::Launch(alloc::string::String::from(path)),
    };

    Some(KeyboardShortcut {
        modifiers: mods,
        key_code: kc,
        action,
    })
}

fn parse_key_name(name: &str) -> Option<u32> {
    if name.len() == 1 {
        let ch = name.as_bytes()[0];
        match ch {
            b'A'..=b'Z' | b'a'..=b'z' => {
                return Some(letter_to_scancode(ch.to_ascii_uppercase())? as u32)
            }
            b'0'..=b'9' => return Some(digit_to_scancode(ch)? as u32),
            _ => return None,
        }
    }

    match name {
        "Enter" | "Return" => Some(KEY_ENTER),
        "Space" => Some(KEY_SPACE),
        "Tab" => Some(KEY_TAB),
        "Escape" | "Esc" => Some(KEY_ESCAPE),
        "Delete" | "Del" => Some(KEY_DELETE),
        "Backspace" => Some(KEY_BACKSPACE),
        "Home" => Some(KEY_HOME),
        "End" => Some(KEY_END),
        "PageUp" => Some(KEY_PAGE_UP),
        "PageDown" => Some(KEY_PAGE_DOWN),
        "Up" => Some(KEY_UP),
        "Down" => Some(KEY_DOWN),
        "Left" => Some(KEY_LEFT),
        "Right" => Some(KEY_RIGHT),
        "F1" => Some(KEY_F1),
        "F2" => Some(KEY_F2),
        "F3" => Some(KEY_F3),
        "F4" => Some(KEY_F4),
        "F5" => Some(KEY_F5),
        "F6" => Some(KEY_F6),
        "F7" => Some(KEY_F7),
        "F8" => Some(KEY_F8),
        "F9" => Some(KEY_F9),
        "F10" => Some(KEY_F10),
        "F11" => Some(KEY_F11),
        "F12" => Some(KEY_F12),
        _ => None,
    }
}

fn letter_to_scancode(ch: u8) -> Option<u8> {
    match ch {
        b'Q' => Some(0x10),
        b'W' => Some(0x11),
        b'E' => Some(0x12),
        b'R' => Some(0x13),
        b'T' => Some(0x14),
        b'Y' => Some(0x15),
        b'U' => Some(0x16),
        b'I' => Some(0x17),
        b'O' => Some(0x18),
        b'P' => Some(0x19),
        b'A' => Some(0x1E),
        b'S' => Some(0x1F),
        b'D' => Some(0x20),
        b'F' => Some(0x21),
        b'G' => Some(0x22),
        b'H' => Some(0x23),
        b'J' => Some(0x24),
        b'K' => Some(0x25),
        b'L' => Some(0x26),
        b'Z' => Some(0x2C),
        b'X' => Some(0x2D),
        b'C' => Some(0x2E),
        b'V' => Some(0x2F),
        b'B' => Some(0x30),
        b'N' => Some(0x31),
        b'M' => Some(0x32),
        _ => None,
    }
}

fn digit_to_scancode(ch: u8) -> Option<u8> {
    match ch {
        b'1' => Some(0x02),
        b'2' => Some(0x03),
        b'3' => Some(0x04),
        b'4' => Some(0x05),
        b'5' => Some(0x06),
        b'6' => Some(0x07),
        b'7' => Some(0x08),
        b'8' => Some(0x09),
        b'9' => Some(0x0A),
        b'0' => Some(0x0B),
        _ => None,
    }
}
